use std::time::Duration;

use anyhow::Result;
use serde_json::json;

use super::*;
use crate::testing::fake_sources;

/// The one model the fake sampler is told about.
fn gemma() -> Vec<ServedModel> {
    vec![ServedModel {
        id: "Gemma-4-26B-A4B-NVFP4".to_string(),
        engine: Some("tensorrt-edgellm".to_string()),
    }]
}

#[tokio::test]
async fn a_sample_reads_every_source() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let monitor = Monitor::new(fake_sources(folder.path())?);

    let report = monitor.report(gemma).await;

    assert!(report.sampled_at_unix_ms > 0);
    assert_eq!(
        report,
        Report {
            host: Some("thor".to_string()),
            sampled_at_unix_ms: report.sampled_at_unix_ms,
            gpu: Gpu {
                name: Some("NVIDIA Thor".to_string()),
                utilization_percent: Some(0.0),
                memory_used_bytes: Some(750 * 1024),
                memory_total_bytes: Some(1000 * 1024),
                memory_is_unified: Some(true),
                power_watts: Some(4.37),
                temperature_celsius: Some(41.0),
            },
            cpu: Cpu {
                utilization_percent: None,
                cores: Some(1),
            },
            models: gemma(),
        }
    );

    Ok(())
}

#[tokio::test]
async fn a_second_request_within_two_seconds_gets_the_same_sample() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let monitor = Monitor::new(fake_sources(folder.path())?);

    let first = monitor.report(gemma).await;
    let second = monitor.report(Vec::new).await;

    assert_eq!(first, second);

    Ok(())
}

#[tokio::test]
async fn sources_that_are_missing_measure_nothing() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let nowhere = folder.path().join("missing");
    let sources = Sources {
        gpu_command: vec![nowhere.display().to_string()],
        gpu_timeout: Duration::from_secs(5),
        meminfo: nowhere.clone(),
        stat: nowhere.clone(),
        cpu_interval: Duration::ZERO,
        hostname: nowhere,
    };

    let report = Monitor::new(sources).report(Vec::new).await;

    assert_eq!(
        report,
        Report {
            sampled_at_unix_ms: report.sampled_at_unix_ms,
            ..Report::default()
        }
    );

    Ok(())
}

#[tokio::test]
async fn a_gpu_command_that_fails_or_hangs_prints_nothing() {
    let command =
        |words: &[&str]| -> Vec<String> { words.iter().map(ToString::to_string).collect() };
    let short = Duration::from_millis(100);

    assert_eq!(
        command_output(&command(&["echo", "x"]), short).await,
        Some("x\n".to_string())
    );
    assert_eq!(command_output(&command(&["false"]), short).await, None);
    assert_eq!(command_output(&command(&["sleep", "5"]), short).await, None);
    assert_eq!(command_output(&[], short).await, None);
}

#[test]
fn the_real_sources_are_nvidia_smi_and_proc() {
    let sources = Sources::this_machine();

    assert_eq!(
        sources.gpu_command.first().map(String::as_str),
        Some("nvidia-smi")
    );
    assert_eq!(sources.meminfo, PathBuf::from("/proc/meminfo"));
    assert_eq!(sources.stat, PathBuf::from("/proc/stat"));
}

#[test]
fn a_listed_model_needs_an_id() {
    let listed = served_model(&json!({ "id": "qwen", "owned_by": "llamacpp" }));
    let unowned = served_model(&json!({ "id": "plain" }));

    assert_eq!(
        listed,
        Some(ServedModel {
            id: "qwen".to_string(),
            engine: Some("llamacpp".to_string()),
        })
    );
    assert_eq!(unowned.and_then(|model| model.engine), None);
    assert_eq!(served_model(&json!({ "object": "model" })), None);
}

#[test]
fn a_time_before_1970_is_zero() {
    let before = UNIX_EPOCH.checked_sub(Duration::from_secs(1));

    assert_eq!(before.map(unix_ms), Some(0));
    assert_eq!(unix_ms(UNIX_EPOCH + Duration::from_millis(1500)), 1500);
}
