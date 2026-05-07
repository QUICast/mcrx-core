use mcrx_core::{Context, SubscriptionConfig, TokioSubscription};
use std::env;
use std::net::IpAddr;
use std::process;

const MAX_PREVIEW_LEN: usize = 64;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("mcrx-tokio-recv: {err}");
        process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 || args.len() > 5 {
        print_usage(&args[0]);
        return Err("invalid arguments".to_string());
    }

    let group = parse_ip("group", &args[1])?;
    let dst_port = parse_port(&args[2])?;

    let source = if args.len() >= 4 {
        Some(parse_ip("source", &args[3])?)
    } else {
        None
    };

    let interface = if args.len() >= 5 {
        Some(parse_ip("interface", &args[4])?)
    } else {
        None
    };

    if !group.is_multicast() {
        return Err(format!("group address {group} is not multicast"));
    }

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

    let subscription = ctx
        .take_subscription(subscription_id)
        .ok_or_else(|| "failed to extract subscription from context".to_string())?;
    let mut subscription = TokioSubscription::new(subscription)
        .map_err(|err| format!("failed to create tokio adapter: {err}"))?;

    println!("mcrx-tokio-recv ready");
    println!("  group:      {group}");
    println!("  dst_port:   {dst_port}");
    println!("  source:     {}", source_string(source));
    println!("  interface:  {}", interface_string(interface));
    println!("  sub_id:     {}", subscription_id.0);
    println!();
    println!("waiting for packets with Tokio ...");

    loop {
        tokio::select! {
            result = subscription.recv_with_metadata() => {
                let packet = result.map_err(|err| format!("receive failed: {err}"))?;
                println!(
                    "[recv] sub={} src={} group={} dst_port={} len={}",
                    packet.packet.subscription_id.0,
                    packet.packet.source,
                    packet.packet.group,
                    packet.packet.dst_port,
                    packet.packet.payload.len()
                );
                println!(
                    "       meta: socket_local_addr={:?} configured_interface={:?} destination_local_ip={:?} ingress_ifindex={:?}",
                    packet.metadata.socket_local_addr,
                    packet.metadata.configured_interface,
                    packet.metadata.destination_local_ip,
                    packet.metadata.ingress_interface_index
                );
                println!("       payload: {}", format_payload(&packet.packet.payload));
            }
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|err| format!("failed to wait for Ctrl-C: {err}"))?;
                println!("shutting down");
                return Ok(());
            }
        }
    }
}

fn parse_ip(name: &str, value: &str) -> Result<IpAddr, String> {
    value
        .parse::<IpAddr>()
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

fn print_usage(program: &str) {
    eprintln!("usage: {program} <group> <dst_port> [source] [interface]");
    eprintln!("  group      multicast group, e.g. 239.1.2.3 or ff01::1234");
    eprintln!("  dst_port   destination UDP port");
    eprintln!("  source     optional source for SSM");
    eprintln!("  interface  optional local interface address");
}
