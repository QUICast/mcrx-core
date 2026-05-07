#[path = "common/recv_args.rs"]
mod recv_args;

use mcrx_core::{Context, SubscriptionConfig};
use std::env;
use std::net::IpAddr;
use std::process;
use std::thread;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_PREVIEW_LEN: usize = 64;

fn main() {
    if let Err(err) = run() {
        eprintln!("mcrx-recv-meta: {err}");
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

    println!("mcrx-recv-meta ready");
    println!("  group:      {group}");
    println!("  dst_port:   {dst_port}");
    println!("  source:     {}", source_string(source));
    println!("  interface:  {}", interface_string(interface));
    println!("  sub_id:     {}", subscription_id.0);
    println!();
    println!("waiting for packets with rich metadata ...");

    loop {
        match ctx
            .try_recv_any_with_metadata()
            .map_err(|err| format!("receive failed: {err}"))?
        {
            Some(packet) => {
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
            None => thread::sleep(POLL_INTERVAL),
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

fn print_usage(program: &str) {
    eprintln!("usage: {program} <group> <dst_port> [source] [interface]");
    eprintln!("       {program} <group> <dst_port> [--source <source>] [--interface <interface>]");
    eprintln!("  group      multicast group, e.g. 239.1.2.3 or ff01::1234");
    eprintln!("  dst_port   destination UDP port");
    eprintln!("  source     optional source for SSM");
    eprintln!("  interface  optional local interface address");
    eprintln!("examples:");
    eprintln!("  {program} ff01::1234 5000 --interface ::1");
    eprintln!("  {program} ff12::1234 5000 --interface fe80::1");
}
