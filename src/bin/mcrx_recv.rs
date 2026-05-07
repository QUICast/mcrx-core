#[path = "common/recv_args.rs"]
mod recv_args;

use mcrx_core::{Context, SubscriptionConfig};
#[cfg(feature = "metrics")]
use mcrx_core::{
    ContextMetricsDelta, ContextMetricsSampler, ContextMetricsSnapshot, HardwareMetricsDelta,
    HardwareMetricsSampler, HardwareMetricsSnapshot,
};
use std::env;
#[cfg(feature = "metrics")]
use std::fs::OpenOptions;
#[cfg(feature = "metrics")]
use std::io::Write;
use std::net::IpAddr;
#[cfg(feature = "metrics")]
use std::path::PathBuf;
use std::process;
#[cfg(feature = "metrics")]
use std::sync::Once;
use std::thread;
use std::time::Duration;
#[cfg(feature = "metrics")]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_PREVIEW_LEN: usize = 64;

fn main() {
    if let Err(err) = run() {
        eprintln!("mcrx-recv: {err}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        print_usage(&args[0]);
        return Err("invalid arguments".to_string());
    }

    let parsed = match recv_args::parse_receive_cli_args(&args) {
        Ok(parsed) => parsed,
        Err(err) => {
            print_usage(&args[0]);
            return Err(err);
        }
    };
    let group = parsed.group;
    let dst_port = parsed.dst_port;
    let source = parsed.source;
    let interface = parsed.interface;

    let mut config = match source {
        Some(source) => SubscriptionConfig::ssm_ip(group, source, dst_port),
        None => SubscriptionConfig::asm_ip(group, dst_port),
    };
    config.interface = interface;

    let mut ctx = Context::new();
    let subscription_id = ctx
        .add_subscription(config)
        .map_err(|err| format!("failed to add subscription: {err}"))?;
    ctx.join_subscription(subscription_id)
        .map_err(|err| format!("failed to join subscription: {err}"))?;

    println!("mcrx-recv ready");
    println!("  group:      {group}");
    println!("  dst_port:   {dst_port}");
    println!("  source:     {}", source_string(source));
    println!("  interface:  {}", interface_string(interface));
    println!("  sub_id:     {}", subscription_id.0);

    println!();
    println!("waiting for packets ...");

    #[cfg(feature = "metrics")]
    let summary_interval = summary_interval_from_env();
    #[cfg(feature = "metrics")]
    let summary_file = summary_file_from_env();
    #[cfg(feature = "metrics")]
    let mut metrics_sampler = ContextMetricsSampler::new();
    #[cfg(feature = "metrics")]
    let _ = metrics_sampler.sample(ctx.metrics_snapshot());
    #[cfg(feature = "metrics")]
    let mut hardware_metrics_sampler = init_hardware_metrics_sampler()?;
    #[cfg(feature = "metrics")]
    let mut next_summary_at = summary_interval.map(|interval| Instant::now() + interval);

    loop {
        match ctx
            .try_recv_any()
            .map_err(|err| format!("receive failed: {err}"))?
        {
            Some(packet) => {
                println!(
                    "[recv] sub={} src={} group={} dst_port={} len={}",
                    packet.subscription_id.0,
                    packet.source,
                    packet.group,
                    packet.dst_port,
                    packet.payload.len()
                );

                println!("       payload: {}", format_payload(&packet.payload));
            }
            None => {
                thread::sleep(POLL_INTERVAL);
            }
        }
        #[cfg(feature = "metrics")]
        if let (Some(interval), Some(deadline)) = (summary_interval, next_summary_at)
            && Instant::now() >= deadline
        {
            let snapshot = ctx.metrics_snapshot();

            if let Some(delta) = metrics_sampler.sample(snapshot.clone()) {
                if let Some(path) = &summary_file {
                    write_metrics_summary_jsonl(&snapshot, &delta, path)
                        .map_err(|err| format!("failed to write metrics summary: {err}"))?;
                } else {
                    print_metrics_summary(&snapshot, &delta);
                }
            }

            if let Some(hardware_sampler) = hardware_metrics_sampler.as_mut()
                && let Some(hardware_snapshot) = capture_hardware_metrics_snapshot()?
            {
                if let Some(delta) = hardware_sampler.sample(hardware_snapshot.clone()) {
                    if let Some(path) = &summary_file {
                        write_hardware_metrics_summary_jsonl(&hardware_snapshot, &delta, path)
                            .map_err(|err| {
                                format!("failed to write hardware metrics summary: {err}")
                            })?;
                    } else {
                        print_hardware_metrics_summary(&hardware_snapshot, &delta);
                    }
                }
            }

            next_summary_at = Some(Instant::now() + interval);
        }
    }
}

fn source_string(source: Option<IpAddr>) -> String {
    match source {
        Some(source) => source.to_string(),
        None => "any".to_string(),
    }
}

fn interface_string(interface: Option<IpAddr>) -> String {
    match interface {
        Some(interface) => interface.to_string(),
        None => "default".to_string(),
    }
}

fn format_payload(payload: &[u8]) -> String {
    match std::str::from_utf8(payload) {
        Ok(text) => truncate_preview(text, MAX_PREVIEW_LEN),
        Err(_) => {
            let preview_len = payload.len().min(16);
            let hex_preview = payload[..preview_len]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");

            if payload.len() > preview_len {
                format!("0x{hex_preview} ... ({} bytes total)", payload.len())
            } else {
                format!("0x{hex_preview}")
            }
        }
    }
}

fn truncate_preview(text: &str, max_len: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_len {
        return text.to_string();
    }

    let truncated: String = text.chars().take(max_len).collect();
    format!("{truncated}...")
}

#[cfg(feature = "metrics")]
fn summary_interval_from_env() -> Option<Duration> {
    let raw = env::var("MCRX_METRICS_SUMMARY_SECS").ok()?;
    let secs = raw.parse::<u64>().ok()?;
    if secs == 0 {
        None
    } else {
        Some(Duration::from_secs(secs))
    }
}

#[cfg(feature = "metrics")]
fn summary_file_from_env() -> Option<PathBuf> {
    let raw = env::var("MCRX_METRICS_SUMMARY_FILE").ok()?;
    if raw.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(raw))
    }
}

#[cfg(feature = "metrics")]
fn init_hardware_metrics_sampler() -> Result<Option<HardwareMetricsSampler>, String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let mut sampler = HardwareMetricsSampler::new();
        let _ = sampler.sample(
            HardwareMetricsSnapshot::capture_current_process()
                .map_err(|err| format!("failed to capture initial hardware metrics: {err}"))?,
        );
        Ok(Some(sampler))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok(None)
    }
}

#[cfg(feature = "metrics")]
fn capture_hardware_metrics_snapshot() -> Result<Option<HardwareMetricsSnapshot>, String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        HardwareMetricsSnapshot::capture_current_process()
            .map(Some)
            .map_err(|err| format!("failed to capture hardware metrics: {err}"))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok(None)
    }
}

#[cfg(feature = "metrics")]
fn hardware_summary_file_path(network_path: &PathBuf) -> PathBuf {
    let parent = network_path.parent().map(PathBuf::from).unwrap_or_default();
    let stem = network_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("metrics");
    let extension = network_path.extension().and_then(|ext| ext.to_str());

    let file_name = match extension {
        Some(ext) if !ext.is_empty() => format!("{stem}_hardware.{ext}"),
        _ => format!("{stem}_hardware"),
    };

    parent.join(file_name)
}

#[cfg(feature = "metrics")]
fn unix_timestamp_secs(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(feature = "metrics")]
fn write_metrics_summary_jsonl(
    snapshot: &ContextMetricsSnapshot,
    delta: &ContextMetricsDelta,
    path: &PathBuf,
) -> Result<(), std::io::Error> {
    let timestamp_secs = unix_timestamp_secs(snapshot.captured_at);

    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path);
    });

    let line = format!(
        concat!(
            "{{",
            "\"ts\":{},",
            "\"interval_secs\":{},",
            "\"active_subscriptions\":{},",
            "\"joined_subscriptions\":{},",
            "\"packets_received_total\":{},",
            "\"bytes_received_total\":{},",
            "\"packets_received_delta\":{},",
            "\"bytes_received_delta\":{},",
            "\"would_block_count_total\":{},",
            "\"would_block_count_delta\":{},",
            "\"receive_errors_total\":{},",
            "\"receive_errors_delta\":{},",
            "\"join_count_total\":{},",
            "\"join_count_delta\":{},",
            "\"leave_count_total\":{},",
            "\"leave_count_delta\":{},",
            "\"batch_calls_total\":{},",
            "\"batch_calls_delta\":{},",
            "\"batch_packets_received_total\":{},",
            "\"batch_packets_received_delta\":{},",
            "\"packets_per_sec\":{},",
            "\"bytes_per_sec\":{},",
            "\"would_block_per_sec\":{},",
            "\"receive_errors_per_sec\":{}",
            "}}\n"
        ),
        timestamp_secs,
        delta.interval_secs,
        snapshot.active_subscriptions,
        snapshot.joined_subscriptions,
        snapshot.total_packets_received,
        snapshot.total_bytes_received,
        delta.packets_received,
        delta.bytes_received,
        snapshot.total_would_block_count,
        delta.would_block_count,
        snapshot.total_receive_errors,
        delta.receive_errors,
        snapshot.total_join_count,
        delta.join_count,
        snapshot.total_leave_count,
        delta.leave_count,
        snapshot.batch_calls,
        delta.batch_calls,
        snapshot.batch_packets_received,
        delta.batch_packets_received,
        delta.packets_per_sec(),
        delta.bytes_per_sec(),
        delta.would_block_per_sec(),
        delta.receive_errors_per_sec(),
    );

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

#[cfg(feature = "metrics")]
fn write_hardware_metrics_summary_jsonl(
    snapshot: &HardwareMetricsSnapshot,
    delta: &HardwareMetricsDelta,
    network_path: &PathBuf,
) -> Result<(), std::io::Error> {
    let timestamp_secs = unix_timestamp_secs(snapshot.captured_at);
    let hardware_path = hardware_summary_file_path(network_path);

    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&hardware_path);
    });

    let line = format!(
        concat!(
            "{{",
            "\"ts\":{},",
            "\"interval_secs\":{},",
            "\"cpu_user_secs\":{},",
            "\"cpu_system_secs\":{},",
            "\"cpu_total_secs\":{},",
            "\"cpu_util_percent\":{},",
            "\"rss_bytes\":{},",
            "\"virtual_memory_bytes\":{},",
            "\"thread_count\":{},",
            "\"open_fds\":{},",
            "\"page_faults_minor\":{},",
            "\"page_faults_major\":{},",
            "\"ctx_switches_voluntary\":{},",
            "\"ctx_switches_involuntary\":{}",
            "}}\n"
        ),
        timestamp_secs,
        delta.interval_secs,
        delta.cpu_user_secs,
        delta.cpu_system_secs,
        delta.cpu_total_secs,
        delta.cpu_util_percent,
        delta.rss_bytes,
        delta.virtual_memory_bytes,
        delta.thread_count,
        delta.open_fds,
        delta.page_faults_minor,
        delta.page_faults_major,
        delta.ctx_switches_voluntary,
        delta.ctx_switches_involuntary,
    );

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(hardware_path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

#[cfg(feature = "metrics")]
fn print_metrics_summary(snapshot: &ContextMetricsSnapshot, delta: &ContextMetricsDelta) {
    println!("[metrics]");
    println!("  interval_secs:         {:.3}", delta.interval_secs);
    println!("  active_subscriptions:  {}", snapshot.active_subscriptions);
    println!("  joined_subscriptions:  {}", snapshot.joined_subscriptions);
    println!(
        "  packets_received_total: {}",
        snapshot.total_packets_received
    );
    println!("  packets_received_delta: {}", delta.packets_received);
    println!(
        "  bytes_received_total:   {}",
        snapshot.total_bytes_received
    );
    println!("  bytes_received_delta:   {}", delta.bytes_received);
    println!(
        "  would_block_count_total: {}",
        snapshot.total_would_block_count
    );
    println!("  would_block_count_delta: {}", delta.would_block_count);
    println!(
        "  receive_errors_total:   {}",
        snapshot.total_receive_errors
    );
    println!("  receive_errors_delta:   {}", delta.receive_errors);
    println!("  join_count_total:       {}", snapshot.total_join_count);
    println!("  join_count_delta:       {}", delta.join_count);
    println!("  leave_count_total:      {}", snapshot.total_leave_count);
    println!("  leave_count_delta:      {}", delta.leave_count);
    println!("  batch_calls_total:      {}", snapshot.batch_calls);
    println!("  batch_calls_delta:      {}", delta.batch_calls);
    println!(
        "  batch_packets_received_total: {}",
        snapshot.batch_packets_received
    );
    println!(
        "  batch_packets_received_delta: {}",
        delta.batch_packets_received
    );
    println!("  packets_per_sec:       {:.3}", delta.packets_per_sec());
    println!("  bytes_per_sec:         {:.3}", delta.bytes_per_sec());
    println!(
        "  would_block_per_sec:   {:.3}",
        delta.would_block_per_sec()
    );
    println!(
        "  recv_errors_per_sec:   {:.3}",
        delta.receive_errors_per_sec()
    );
}

#[cfg(feature = "metrics")]
fn print_hardware_metrics_summary(
    _snapshot: &HardwareMetricsSnapshot,
    delta: &HardwareMetricsDelta,
) {
    println!("[hardware]");
    println!("  interval_secs:              {:.3}", delta.interval_secs);
    println!("  cpu_user_secs:              {:.6}", delta.cpu_user_secs);
    println!("  cpu_system_secs:            {:.6}", delta.cpu_system_secs);
    println!("  cpu_total_secs:             {:.6}", delta.cpu_total_secs);
    println!(
        "  cpu_util_percent:           {:.3}",
        delta.cpu_util_percent
    );
    println!("  rss_bytes:                  {}", delta.rss_bytes);
    println!(
        "  virtual_memory_bytes:       {}",
        delta.virtual_memory_bytes
    );
    println!("  thread_count:               {}", delta.thread_count);
    println!("  open_fds:                   {}", delta.open_fds);
    println!("  page_faults_minor:          {}", delta.page_faults_minor);
    println!("  page_faults_major:          {}", delta.page_faults_major);
    println!(
        "  ctx_switches_voluntary:     {}",
        delta.ctx_switches_voluntary
    );
    println!(
        "  ctx_switches_involuntary:   {}",
        delta.ctx_switches_involuntary
    );
}

fn print_usage(program: &str) {
    eprintln!("Usage:");
    eprintln!("  {program} <group> <dst_port> [source] [interface]");
    eprintln!("  {program} <group> <dst_port> [--source <source>] [--interface <interface>]");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  {program} 239.1.2.3 5000");
    eprintln!("  {program} 232.1.2.3 5000 192.168.1.10");
    eprintln!("  {program} 232.1.2.3 5000 192.168.1.10 192.168.1.20");
    eprintln!("  {program} ff01::1234 5000");
    eprintln!("  {program} ff01::1234 5000 --interface ::1");
    eprintln!();
    eprintln!("Notes:");
    eprintln!("  - omit <source> for ASM");
    eprintln!("  - provide <source> for SSM");
    eprintln!("  - use --interface for ASM when you want to set the join interface without also setting a source");
    eprintln!("  - <interface> selects the local join interface address");

    #[cfg(feature = "metrics")]
    {
        eprintln!();
        eprintln!("Metrics (when built with --features metrics):");
        eprintln!("  MCRX_METRICS_SUMMARY_SECS=<n>   emit a delta metrics summary every n seconds");
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        eprintln!(
            "  MCRX_METRICS_SUMMARY_FILE=<p>   write network metrics with explicit *_total and *_delta fields to <p> and hardware metrics to a sibling *_hardware file"
        );
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        eprintln!(
            "  MCRX_METRICS_SUMMARY_FILE=<p>   write network metrics with explicit *_total and *_delta fields to <p>"
        );
    }
}

#[cfg(all(test, feature = "metrics"))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    fn unique_temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos();
        std::env::temp_dir().join(format!("mcrx_recv_{name}_{unique}.jsonl"))
    }

    #[test]
    fn metrics_jsonl_uses_explicit_total_and_delta_field_names() {
        let path = unique_temp_path("metrics_schema");

        let snapshot = ContextMetricsSnapshot {
            subscriptions_added: 1,
            subscriptions_removed: 0,
            active_subscriptions: 1,
            joined_subscriptions: 1,
            total_packets_received: 10,
            total_bytes_received: 1000,
            total_would_block_count: 4,
            total_receive_errors: 2,
            total_join_count: 3,
            total_leave_count: 1,
            batch_calls: 7,
            batch_packets_received: 10,
            captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(123),
        };

        let delta = ContextMetricsDelta {
            interval_secs: 2.0,
            packets_received: 5,
            bytes_received: 600,
            would_block_count: 1,
            receive_errors: 1,
            join_count: 1,
            leave_count: 0,
            batch_calls: 2,
            batch_packets_received: 5,
        };

        write_metrics_summary_jsonl(&snapshot, &delta, &path).unwrap();

        let line = fs::read_to_string(&path).unwrap();

        assert!(line.contains("\"packets_received_total\":10"));
        assert!(line.contains("\"bytes_received_total\":1000"));
        assert!(line.contains("\"packets_received_delta\":5"));
        assert!(line.contains("\"bytes_received_delta\":600"));
        assert!(line.contains("\"would_block_count_total\":4"));
        assert!(line.contains("\"would_block_count_delta\":1"));
        assert!(line.contains("\"receive_errors_total\":2"));
        assert!(line.contains("\"receive_errors_delta\":1"));
        assert!(line.contains("\"join_count_total\":3"));
        assert!(line.contains("\"join_count_delta\":1"));
        assert!(line.contains("\"leave_count_total\":1"));
        assert!(line.contains("\"leave_count_delta\":0"));
        assert!(line.contains("\"batch_calls_total\":7"));
        assert!(line.contains("\"batch_calls_delta\":2"));
        assert!(line.contains("\"batch_packets_received_total\":10"));
        assert!(line.contains("\"batch_packets_received_delta\":5"));
        assert!(!line.contains("\"packets_received\":"));
        assert!(!line.contains("\"bytes_received\":"));
        assert!(!line.contains("\"would_block_count\":"));
        assert!(!line.contains("\"receive_errors\":"));
        assert!(!line.contains("\"join_count\":"));
        assert!(!line.contains("\"leave_count\":"));
        assert!(!line.contains("\"batch_calls\":"));
        assert!(!line.contains("\"batch_packets_received\":"));

        let _ = fs::remove_file(path);
    }
}
