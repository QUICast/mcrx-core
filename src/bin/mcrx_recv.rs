use mcrx_core::{Context, SourceFilter, SubscriptionConfig};
use std::env;
use std::net::Ipv4Addr;
use std::process;
use std::thread;
use std::time::Duration;

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

    if args.len() < 3 || args.len() > 5 {
        print_usage(&args[0]);
        return Err("invalid arguments".to_string());
    }

    let group = parse_ipv4("group", &args[1])?;
    let dst_port = parse_port(&args[2])?;

    let source = if args.len() >= 4 {
        Some(parse_ipv4("source", &args[3])?)
    } else {
        None
    };

    let interface = if args.len() >= 5 {
        Some(parse_ipv4("interface", &args[4])?)
    } else {
        None
    };

    if !group.is_multicast() {
        return Err(format!("group address {group} is not multicast"));
    }

    let source_filter = match source {
        Some(source) => SourceFilter::Source(source),
        None => SourceFilter::Any,
    };

    let config = SubscriptionConfig {
        group,
        source: source_filter,
        dst_port,
        interface,
    };

    let mut ctx = Context::new();
    let subscription_id = ctx
        .add_subscription(config)
        .map_err(|err| format!("failed to add subscription: {err}"))?;
    ctx.join_subscription(subscription_id).unwrap();

    println!("mcrx-recv ready");
    println!("  group:      {group}");
    println!("  dst_port:   {dst_port}");
    println!("  source:     {}", source_string(source));
    println!("  interface:  {}", interface_string(interface));
    println!("  sub_id:     {}", subscription_id.0);

    println!();
    println!("waiting for packets ...");

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
    }
}

fn parse_ipv4(name: &str, value: &str) -> Result<Ipv4Addr, String> {
    value
        .parse::<Ipv4Addr>()
        .map_err(|err| format!("invalid {name} '{value}': {err}"))
}

fn parse_port(value: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|err| format!("invalid dst_port '{value}': {err}"))?;

    if port == 0 {
        return Err("dst_port must not be 0".to_string());
    }

    Ok(port)
}

fn source_string(source: Option<Ipv4Addr>) -> String {
    match source {
        Some(source) => source.to_string(),
        None => "any".to_string(),
    }
}

fn interface_string(interface: Option<Ipv4Addr>) -> String {
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

fn print_usage(program: &str) {
    eprintln!("Usage:");
    eprintln!("  {program} <group> <dst_port> [source] [interface]");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  {program} 239.1.2.3 5000");
    eprintln!("  {program} 232.1.2.3 5000 192.168.1.10");
    eprintln!("  {program} 232.1.2.3 5000 192.168.1.10 192.168.1.20");
    eprintln!();
    eprintln!("Notes:");
    eprintln!("  - omit <source> for ASM");
    eprintln!("  - provide <source> for SSM");
    eprintln!("  - <interface> is optional and selects the local join interface");
}
