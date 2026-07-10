use mcrx_core::{RawSubscriptionConfig, SharedRawContext};
use std::env;
use std::error::Error;
use std::net::IpAddr;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let Some(group) = args.next() else {
        print_usage();
        return Ok(());
    };
    let Some(interface) = args.next() else {
        print_usage();
        return Ok(());
    };
    let source = args.next();
    if args.next().is_some() {
        print_usage();
        return Ok(());
    }

    let group: IpAddr = group.parse()?;
    let interface: IpAddr = interface.parse()?;
    let source_address = source.as_deref().map(str::parse).transpose()?;
    let mut config = match source_address {
        Some(source) => RawSubscriptionConfig::ssm_ip(group, source),
        None => RawSubscriptionConfig::asm_ip(group),
    };
    config.interface = Some(interface);

    let mut context = SharedRawContext::new();
    let id = context.add_subscription(config)?;
    context.join_subscription(id)?;

    println!("shared raw capture ready");
    println!("  group:      {group}");
    println!("  interface:  {interface}");
    println!("  source:     {}", source.as_deref().unwrap_or("any"));
    println!("  sub_id:     {}", id.0);

    loop {
        if let Some(packet) = context.try_recv_any()? {
            println!(
                "received {} raw bytes matching {:?}",
                packet.packet.datagram_len(),
                packet.matching_subscription_ids()
            );
        } else {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn print_usage() {
    eprintln!("usage: shared_raw_capture <group> <interface> [source]");
    eprintln!("example: shared_raw_capture 239.1.2.3 192.168.1.20");
    eprintln!("example: shared_raw_capture 232.1.2.3 192.168.1.20 192.168.1.10");
}
