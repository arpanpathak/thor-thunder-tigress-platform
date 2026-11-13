//! The text of `nvidia-smi`, `/proc/meminfo`, `/proc/stat` and the hostname
//! file, turned into numbers. Each function takes the text, so tests can give
//! it fixed input; a value the text does not hold is `None`.

use super::Gpu;

/// What `nvidia-smi` prints for a value it cannot measure, such as GPU
/// memory on a machine whose GPU shares system RAM.
const NOT_AVAILABLE: &str = "[N/A]";

/// Bytes in a MiB, the unit of `nvidia-smi`'s memory fields.
const MIB: u64 = 1 << 20;

/// Bytes in a kB, the unit of `/proc/meminfo`'s fields.
const KIB: u64 = 1 << 10;

/// The system's RAM, from `/proc/meminfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemMemory {
    /// RAM in use: everything but what a new process could still get.
    pub used_bytes: u64,
    /// RAM in all.
    pub total_bytes: u64,
}

/// The CPU time counters of `/proc/stat`'s first line, in clock ticks since
/// boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuTimes {
    /// Time spent on anything but idling and waiting for I/O.
    busy: u64,
    /// All time counted.
    total: u64,
}

/// The first GPU in `nvidia-smi --query-gpu=name,utilization.gpu,memory.used,
/// memory.total,power.draw,temperature.gpu --format=csv,noheader,nounits`.
/// `None` when the first line does not have six fields.
pub fn gpu(output: &str) -> Option<Gpu> {
    let fields: Vec<&str> = output.lines().next()?.split(',').map(str::trim).collect();
    let [name, utilization, used_mib, total_mib, power, temperature] =
        <[&str; 6]>::try_from(fields).ok()?;

    let memory_total_bytes = mebibytes(total_mib);

    Some(Gpu {
        name: known(name).map(ToString::to_string),
        utilization_percent: number(utilization),
        memory_used_bytes: mebibytes(used_mib),
        memory_total_bytes,
        memory_is_unified: memory_total_bytes.map(|_| false),
        power_watts: number(power),
        temperature_celsius: number(temperature),
    })
}

/// `gpu` with the system's RAM as its memory when `nvidia-smi` gave none:
/// a GPU that shares system RAM has no memory of its own to report.
pub fn with_system_memory(gpu: Gpu, system_memory: Option<SystemMemory>) -> Gpu {
    match (gpu.memory_total_bytes, system_memory) {
        (Some(_), _) => gpu,
        (None, Some(memory)) => Gpu {
            memory_used_bytes: Some(memory.used_bytes),
            memory_total_bytes: Some(memory.total_bytes),
            memory_is_unified: Some(true),
            ..gpu
        },
        (None, None) => gpu,
    }
}

/// The RAM in `/proc/meminfo`: `MemTotal`, and in use `MemTotal` less
/// `MemAvailable`.
pub fn system_memory(meminfo: &str) -> Option<SystemMemory> {
    let total = kilobytes(meminfo, "MemTotal:")?;
    let available = kilobytes(meminfo, "MemAvailable:")?;

    Some(SystemMemory {
        used_bytes: total.saturating_sub(available).saturating_mul(KIB),
        total_bytes: total.saturating_mul(KIB),
    })
}

/// How busy the CPU was between two readings of `/proc/stat`, from 0 to 100
/// with one decimal. `None` when either reading has no `cpu` line or no time
/// passed between them.
pub fn busy_percent(before: &str, after: &str) -> Option<f64> {
    let before = cpu_times(before)?;
    let after = cpu_times(after)?;

    let total = after
        .total
        .checked_sub(before.total)
        .filter(|ticks| *ticks > 0)?;
    let busy = after.busy.saturating_sub(before.busy);
    let percent = 100.0 * busy as f64 / total as f64;

    Some((percent * 10.0).round() / 10.0)
}

/// How many `cpuN` lines `/proc/stat` has; `None` when it has none.
pub fn cores(stat: &str) -> Option<u32> {
    let count = stat.lines().filter(|line| is_core(line)).count();

    u32::try_from(count).ok().filter(|count| *count > 0)
}

/// The hostname in the text of `/proc/sys/kernel/hostname`.
pub fn host_name(text: &str) -> Option<String> {
    let name = text.trim();

    (!name.is_empty()).then(|| name.to_string())
}

/// The counters of `/proc/stat`'s first line, `cpu  user nice system idle
/// iowait irq softirq steal …`; the guest counters after them are already
/// part of `user` and `nice`.
fn cpu_times(stat: &str) -> Option<CpuTimes> {
    let counters = cpu_counters(stat)?;
    let [user, nice, system, idle, iowait, irq, softirq, steal] =
        <[u64; 8]>::try_from(counters.get(..8)?).ok()?;

    let waiting = idle + iowait;
    let total = user + nice + system + waiting + irq + softirq + steal;

    Some(CpuTimes {
        busy: total - waiting,
        total,
    })
}

/// Every number on `/proc/stat`'s first line, when it is the `cpu` line.
fn cpu_counters(stat: &str) -> Option<Vec<u64>> {
    stat.lines()
        .next()?
        .strip_prefix("cpu ")?
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()
}

/// Whether a `/proc/stat` line is one core's: `cpu` followed by a digit.
fn is_core(line: &str) -> bool {
    line.strip_prefix("cpu")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|first| first.is_ascii_digit())
}

/// The number after `name` in `/proc/meminfo`, in kB.
fn kilobytes(meminfo: &str, name: &str) -> Option<u64> {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix(name))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// A field `nvidia-smi` measured; `None` for `[N/A]` or an empty field.
fn known(field: &str) -> Option<&str> {
    Some(field).filter(|field| !matches!(*field, "" | NOT_AVAILABLE))
}

/// A field as a finite number; `None` for `[N/A]` or anything else.
fn number(field: &str) -> Option<f64> {
    field.parse().ok().filter(|value: &f64| value.is_finite())
}

/// A field in MiB, as bytes.
fn mebibytes(field: &str) -> Option<u64> {
    field.parse::<u64>().ok()?.checked_mul(MIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `nvidia-smi` prints on the Thor, whose GPU shares system RAM.
    const THOR: &str = "NVIDIA Thor, 0, [N/A], [N/A], 4.37, 41\n";

    /// What `nvidia-smi` prints on a machine with a discrete GPU.
    const DISCRETE: &str = "NVIDIA GeForce RTX 4090, 37, 20480, 24564, 312.55, 63\n";

    /// The start of the Thor's `/proc/meminfo`.
    const MEMINFO: &str = "MemTotal:       128766588 kB\n\
                           MemFree:         5612616 kB\n\
                           MemAvailable:   38242884 kB\n";

    /// Two readings of a two-core `/proc/stat`, 400 ticks apart, 100 of
    /// them busy and 300 idle or waiting for I/O.
    const STAT_BEFORE: &str = "cpu  1000 0 500 8000 100 50 50 0 0 0\n\
                               cpu0 500 0 250 4000 50 25 25 0 0 0\n\
                               cpu1 500 0 250 4000 50 25 25 0 0 0\n\
                               intr 12345\n";
    const STAT_AFTER: &str = "cpu  1060 0 530 8250 150 55 55 0 0 0\n\
                              cpu0 530 0 265 4125 75 28 27 0 0 0\n\
                              cpu1 530 0 265 4125 75 27 28 0 0 0\n";

    #[test]
    fn the_thor_line_has_no_gpu_memory() {
        let thor = gpu(THOR);

        assert_eq!(
            thor,
            Some(Gpu {
                name: Some("NVIDIA Thor".to_string()),
                utilization_percent: Some(0.0),
                memory_used_bytes: None,
                memory_total_bytes: None,
                memory_is_unified: None,
                power_watts: Some(4.37),
                temperature_celsius: Some(41.0),
            })
        );
    }

    #[test]
    fn a_discrete_gpu_reports_its_memory_in_mib() {
        let discrete = gpu(DISCRETE);

        assert_eq!(
            discrete,
            Some(Gpu {
                name: Some("NVIDIA GeForce RTX 4090".to_string()),
                utilization_percent: Some(37.0),
                memory_used_bytes: Some(20_480 * MIB),
                memory_total_bytes: Some(24_564 * MIB),
                memory_is_unified: Some(false),
                power_watts: Some(312.55),
                temperature_celsius: Some(63.0),
            })
        );
    }

    #[test]
    fn a_line_without_six_fields_is_no_gpu() {
        assert_eq!(gpu(""), None);
        assert_eq!(gpu("No devices were found\n"), None);
        assert_eq!(gpu("NVIDIA Thor, 0, 1, 2, 3, 4, 5\n"), None);
    }

    #[test]
    fn unmeasured_fields_are_none() {
        let unknown = gpu("[N/A], [N/A], [N/A], [N/A], [N/A], nan\n");

        assert_eq!(unknown, Some(Gpu::default()));
    }

    #[test]
    fn the_thor_uses_system_ram_as_gpu_memory() {
        let memory = system_memory(MEMINFO);
        let thor = gpu(THOR).map(|thor| with_system_memory(thor, memory));

        assert_eq!(
            memory,
            Some(SystemMemory {
                used_bytes: (128_766_588 - 38_242_884) * KIB,
                total_bytes: 128_766_588 * KIB,
            })
        );
        assert_eq!(
            thor.as_ref().map(|thor| (
                thor.memory_used_bytes,
                thor.memory_total_bytes,
                thor.memory_is_unified
            )),
            Some((Some(92_696_272_896), Some(131_856_986_112), Some(true)))
        );
    }

    #[test]
    fn a_gpu_with_its_own_memory_keeps_it() {
        let memory = system_memory(MEMINFO);
        let discrete = gpu(DISCRETE).map(|discrete| with_system_memory(discrete, memory));

        assert_eq!(discrete, gpu(DISCRETE));
    }

    #[test]
    fn without_meminfo_the_memory_stays_unknown() {
        let thor = gpu(THOR).map(|thor| with_system_memory(thor, system_memory("")));

        assert_eq!(thor, gpu(THOR));
        assert_eq!(system_memory("MemTotal: 10 kB\n"), None);
    }

    #[test]
    fn the_cpu_was_busy_a_quarter_of_the_time() {
        assert_eq!(busy_percent(STAT_BEFORE, STAT_AFTER), Some(25.0));
        assert_eq!(cores(STAT_AFTER), Some(2));
    }

    #[test]
    fn the_cpu_is_unknown_without_two_readings_apart() {
        assert_eq!(busy_percent(STAT_BEFORE, STAT_BEFORE), None);
        assert_eq!(busy_percent(STAT_AFTER, STAT_BEFORE), None);
        assert_eq!(busy_percent("", STAT_AFTER), None);
        assert_eq!(busy_percent(STAT_BEFORE, "cpu  1 2 3\n"), None);
        assert_eq!(busy_percent(STAT_BEFORE, "cpu  1 x 3 4 5 6 7 8\n"), None);
        assert_eq!(cores("cpu  1 2 3\nintr 5\n"), None);
    }

    #[test]
    fn the_hostname_is_the_trimmed_file() {
        assert_eq!(host_name("thor\n"), Some("thor".to_string()));
        assert_eq!(host_name("\n"), None);
    }
}
