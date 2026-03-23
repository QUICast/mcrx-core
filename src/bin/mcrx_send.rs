use std::env;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::process;
use std::thread;
use std::time::Duration;

fn main() {
    if let Err(err) = run() {
        eprintln!("mcrx-send: {err}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 4 || args.len() > 6 {
        print_usage(&args[0]);
        return Err("invalid arguments".to_string());
    }

    let group = parse_ipv4("group", &args[1])?;
    let dst_port = parse_port(&args[2])?;
    let message = args[3].clone();

    let interval_ms = if args.len() >= 5 {
        parse_u64("interval_ms", &args[4])?
    } else {
        0
    };

    let interface = if args.len() >= 6 {
        Some(parse_ipv4("interface", &args[5])?)
    } else {
        None
    };

    if !group.is_multicast() {
        return Err(format!("group address {group} is not multicast"));
    }

    let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .map_err(|err| format!("failed to bind sender socket: {err}"))?;

    sender
        .set_multicast_loop_v4(true)
        .map_err(|err| format!("failed to enable multicast loopback: {err}"))?;

    sender
        .set_multicast_ttl_v4(1)
        .map_err(|err| format!("failed to set multicast TTL: {err}"))?;

    if let Some(interface) = interface {
        set_outgoing_interface(&sender, interface)?;
    }

    let destination = SocketAddrV4::new(group, dst_port);

    println!("mcrx-send ready");
    println!("  group:      {group}");
    println!("  dst_port:   {dst_port}");
    println!("  interface:  {}", interface_string(interface));
    println!("  interval:   {} ms", interval_ms);
    println!("  payload:    {message}");

    if interval_ms == 0 {
        send_once(&sender, destination, message.as_bytes())?;
    } else {
        loop {
            send_once(&sender, destination, message.as_bytes())?;
            thread::sleep(Duration::from_millis(interval_ms));
        }
    }

    Ok(())
}

fn send_once(sender: &UdpSocket, destination: SocketAddrV4, payload: &[u8]) -> Result<(), String> {
    let sent = sender
        .send_to(payload, destination)
        .map_err(|err| format!("failed to send packet: {err}"))?;

    println!("sent {sent} bytes to {destination}");
    Ok(())
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

fn parse_u64(name: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|err| format!("invalid {name} '{value}': {err}"))
}

fn interface_string(interface: Option<Ipv4Addr>) -> String {
    match interface {
        Some(interface) => interface.to_string(),
        None => "default".to_string(),
    }
}

fn print_usage(program: &str) {
    eprintln!("Usage:");
    eprintln!("  {program} <group> <dst_port> <message> [interval_ms] [interface]");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  {program} 239.1.2.3 5000 hello");
    eprintln!("  {program} 239.1.2.3 5000 hello 1000");
    eprintln!("  {program} 232.1.2.3 5000 hello 1000 192.168.1.10");
}

#[cfg(unix)]
fn set_outgoing_interface(sender: &UdpSocket, interface: Ipv4Addr) -> Result<(), String> {
    use socket2::SockRef;

    let socket = SockRef::from(sender);
    socket
        .set_multicast_if_v4(&interface)
        .map_err(|err| format!("failed to set outgoing multicast interface to {interface}: {err}"))
}

#[cfg(not(unix))]
fn set_outgoing_interface(_sender: &UdpSocket, interface: Ipv4Addr) -> Result<(), String> {
    Err(format!(
        "setting outgoing multicast interface is not implemented on this platform: {interface}"
    ))
}
