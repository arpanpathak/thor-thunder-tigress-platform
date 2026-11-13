//! `/v1/system`: the machine's GPU, CPU and served models, for coding tools
//! that show the server's load.
//!
//! A sample runs `nvidia-smi` and reads `/proc`, so it is kept for
//! [`MAX_AGE`] and shared by every request in that time. A value that cannot
//! be measured is `null` in the JSON.

mod readings;

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::Value;
use tokio::{process::Command, sync::Mutex};

use crate::{config::Upstreams, engines::models};

/// How long one sample answers every request.
pub const MAX_AGE: Duration = Duration::from_secs(2);

/// How long `nvidia-smi` may run before it is stopped.
const GPU_TIMEOUT: Duration = Duration::from_secs(2);

/// The time between the two readings of `/proc/stat`.
const CPU_INTERVAL: Duration = Duration::from_millis(250);

/// What `nvidia-smi` is asked for, in the order [`readings::gpu`] reads it.
const NVIDIA_SMI: [&str; 4] = [
    "nvidia-smi",
    "--query-gpu=name,utilization.gpu,memory.used,memory.total,power.draw,temperature.gpu",
    "--format=csv,noheader,nounits",
    "--id=0",
];

/// One answer of `/v1/system`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    /// The machine's hostname.
    pub host: Option<String>,
    /// When the sample was taken, in milliseconds since 1970.
    pub sampled_at_unix_ms: u64,
    /// The first GPU.
    pub gpu: Gpu,
    /// All CPU cores together.
    pub cpu: Cpu,
    /// The models `/v1/models` lists.
    pub models: Vec<ServedModel>,
}

/// The first GPU, as `nvidia-smi` reports it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Gpu {
    /// The GPU's name, such as `NVIDIA Thor`.
    pub name: Option<String>,
    /// How busy the GPU was over the last sample period, from 0 to 100.
    pub utilization_percent: Option<f64>,
    /// Memory in use, in bytes.
    pub memory_used_bytes: Option<u64>,
    /// Memory in all, in bytes.
    pub memory_total_bytes: Option<u64>,
    /// Whether the GPU shares the system's RAM, as on the Thor; the memory
    /// fields are then the system's.
    pub memory_is_unified: Option<bool>,
    /// The power the GPU draws, in watts.
    pub power_watts: Option<f64>,
    /// The GPU's temperature, in degrees Celsius.
    pub temperature_celsius: Option<f64>,
}

/// All CPU cores together.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Cpu {
    /// The share of time the cores were busy between two readings, from 0 to 100.
    pub utilization_percent: Option<f64>,
    /// How many cores the kernel lists.
    pub cores: Option<u32>,
}

/// A model the server answers for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServedModel {
    /// The model's id, as `/v1/models` gives it.
    pub id: String,
    /// The server that runs it, from `/v1/models`' `owned_by`.
    pub engine: Option<String>,
}

/// Where a sample is read from. Tests point these at fixed files and commands.
#[derive(Debug, Clone)]
pub struct Sources {
    /// The program and arguments that print the GPU's CSV line.
    pub gpu_command: Vec<String>,
    /// How long `gpu_command` may run.
    pub gpu_timeout: Duration,
    /// The file in `/proc/meminfo`'s format.
    pub meminfo: PathBuf,
    /// The file in `/proc/stat`'s format.
    pub stat: PathBuf,
    /// The time between the two readings of `stat`.
    pub cpu_interval: Duration,
    /// The file holding the hostname.
    pub hostname: PathBuf,
}

impl Sources {
    /// The real sources on a Linux machine with an NVIDIA GPU.
    #[must_use]
    pub fn this_machine() -> Self {
        Sources {
            gpu_command: NVIDIA_SMI.iter().map(ToString::to_string).collect(),
            gpu_timeout: GPU_TIMEOUT,
            meminfo: PathBuf::from("/proc/meminfo"),
            stat: PathBuf::from("/proc/stat"),
            cpu_interval: CPU_INTERVAL,
            hostname: PathBuf::from("/proc/sys/kernel/hostname"),
        }
    }
}

/// A sample and when it was taken.
#[derive(Debug, Clone)]
struct Snapshot {
    /// When the sample was finished.
    taken: Instant,
    /// The sample itself.
    report: Report,
}

impl Snapshot {
    /// Whether the sample is younger than [`MAX_AGE`].
    fn is_fresh(&self) -> bool {
        self.taken.elapsed() < MAX_AGE
    }
}

/// Takes samples from its [`Sources`] and keeps the latest for [`MAX_AGE`].
#[derive(Debug)]
pub struct Monitor {
    /// Where samples are read from.
    sources: Sources,
    /// The latest sample; `None` until the first request.
    latest: Mutex<Option<Snapshot>>,
}

impl Monitor {
    /// A monitor with no sample yet.
    #[must_use]
    pub fn new(sources: Sources) -> Self {
        Monitor {
            sources,
            latest: Mutex::new(None),
        }
    }

    /// The latest sample, or a new one when it is older than [`MAX_AGE`].
    /// `models` lists the served models; it is blocking code, run on tokio's
    /// blocking pool, and only when a new sample is taken.
    ///
    /// Requests that arrive while a sample is taken wait for it, so
    /// `nvidia-smi` runs at most once per [`MAX_AGE`].
    pub async fn report(
        &self,
        models: impl FnOnce() -> Vec<ServedModel> + Send + 'static,
    ) -> Report {
        let mut latest = self.latest.lock().await;
        let fresh = latest.as_ref().filter(|snapshot| snapshot.is_fresh());

        if let Some(snapshot) = fresh {
            return snapshot.report.clone();
        }

        let report = sample(&self.sources, models).await;

        *latest = Some(Snapshot {
            taken: Instant::now(),
            report: report.clone(),
        });

        report
    }
}

/// The models `/v1/models` lists, by id and engine. Blocking: it asks each
/// engine for its list.
pub fn served_models(upstreams: &Upstreams) -> Vec<ServedModel> {
    models::served(upstreams)
        .iter()
        .filter_map(served_model)
        .collect()
}

/// A listed model's id and `owned_by`; `None` for an entry without an id.
fn served_model(model: &Value) -> Option<ServedModel> {
    let id = model.get("id")?.as_str()?.to_string();
    let engine = model
        .get("owned_by")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    Some(ServedModel { id, engine })
}

/// Reads every source at once and puts the readings together.
async fn sample(
    sources: &Sources,
    models: impl FnOnce() -> Vec<ServedModel> + Send + 'static,
) -> Report {
    let listing = tokio::task::spawn_blocking(models);

    let (gpu_line, meminfo, cpu, hostname) = tokio::join!(
        command_output(&sources.gpu_command, sources.gpu_timeout),
        read_text(&sources.meminfo),
        cpu(&sources.stat, sources.cpu_interval),
        read_text(&sources.hostname),
    );

    let system_memory = meminfo.as_deref().and_then(readings::system_memory);
    let gpu = gpu_line
        .as_deref()
        .and_then(readings::gpu)
        .map(|gpu| readings::with_system_memory(gpu, system_memory))
        .unwrap_or_default();

    Report {
        host: hostname.as_deref().and_then(readings::host_name),
        sampled_at_unix_ms: unix_ms(SystemTime::now()),
        gpu,
        cpu,
        models: listing.await.unwrap_or_default(),
    }
}

/// The CPU's busy share between two readings of `stat`, `interval` apart,
/// and its core count.
async fn cpu(stat: &Path, interval: Duration) -> Cpu {
    let before = read_text(stat).await.unwrap_or_default();

    tokio::time::sleep(interval).await;

    let after = read_text(stat).await.unwrap_or_default();

    Cpu {
        utilization_percent: readings::busy_percent(&before, &after),
        cores: readings::cores(&after),
    }
}

/// What `command` prints, when it starts, finishes within `timeout` and
/// succeeds. A command still running at `timeout` is killed.
async fn command_output(command: &[String], timeout: Duration) -> Option<String> {
    let (program, arguments) = command.split_first()?;
    let running = Command::new(program)
        .args(arguments)
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(timeout, running).await.ok()?.ok()?;

    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The text of the file at `path`, when it can be read.
async fn read_text(path: &Path) -> Option<String> {
    tokio::fs::read_to_string(path).await.ok()
}

/// Milliseconds from 1970 to `time`; 0 for a time before 1970.
fn unix_ms(time: SystemTime) -> u64 {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();

    u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
