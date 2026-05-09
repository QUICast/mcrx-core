use socket2::SockRef;
use std::env;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, UdpSocket};
use std::process;
use std::thread;
use std::time::Duration;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
#[cfg(windows)]
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::{AF_INET6, AF_UNSPEC, SOCKADDR_IN6};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterfaceArg {
    Ip(IpAddr),
    V6Index(u32),
}

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

    let group = parse_ip("group", &args[1])?;
    let dst_port = parse_port(&args[2])?;
    let message = args[3].clone();

    let interval_ms = if args.len() >= 5 {
        parse_u64("interval_ms", &args[4])?
    } else {
        0
    };

    let interface = if args.len() >= 6 {
        Some(parse_interface_arg(group, &args[5])?)
    } else {
        None
    };

    if !group.is_multicast() {
        return Err(format!("group address {group} is not multicast"));
    }

    let (sender, destination) = match group {
        IpAddr::V4(group) => prepare_ipv4_sender(group, dst_port, interface)?,
        IpAddr::V6(group) => prepare_ipv6_sender(group, dst_port, interface)?,
    };

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

fn prepare_ipv4_sender(
    group: Ipv4Addr,
    dst_port: u16,
    interface: Option<InterfaceArg>,
) -> Result<(UdpSocket, SocketAddr), String> {
    let bind_addr = match interface {
        Some(InterfaceArg::Ip(IpAddr::V4(interface))) => SocketAddrV4::new(interface, 0),
        _ => SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
    };

    let sender = UdpSocket::bind(bind_addr)
        .map_err(|err| format!("failed to bind IPv4 sender socket: {err}"))?;

    sender
        .set_multicast_loop_v4(true)
        .map_err(|err| format!("failed to enable IPv4 multicast loopback: {err}"))?;

    sender
        .set_multicast_ttl_v4(1)
        .map_err(|err| format!("failed to set IPv4 multicast TTL: {err}"))?;

    if let Some(interface) = interface {
        set_outgoing_interface_v4(&sender, interface)?;
    }

    Ok((sender, SocketAddr::V4(SocketAddrV4::new(group, dst_port))))
}

fn prepare_ipv6_sender(
    group: Ipv6Addr,
    dst_port: u16,
    interface: Option<InterfaceArg>,
) -> Result<(UdpSocket, SocketAddr), String> {
    let bind_addr = match interface {
        Some(InterfaceArg::Ip(IpAddr::V6(interface))) => {
            let scope_id = if interface.is_unicast_link_local() {
                resolve_ipv6_interface_index(interface)?
            } else {
                0
            };
            SocketAddrV6::new(interface, 0, 0, scope_id)
        }
        _ => SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0),
    };

    let sender = UdpSocket::bind(bind_addr)
        .map_err(|err| format!("failed to bind IPv6 sender socket: {err}"))?;

    sender
        .set_multicast_loop_v6(true)
        .map_err(|err| format!("failed to enable IPv6 multicast loopback: {err}"))?;

    let socket = SockRef::from(&sender);
    socket
        .set_multicast_hops_v6(1)
        .map_err(|err| format!("failed to set IPv6 multicast hops: {err}"))?;

    let scope_id = match interface {
        Some(interface) => {
            let scope_id = resolve_ipv6_interface_arg(interface)?;
            if scope_id != 0 {
                socket.set_multicast_if_v6(scope_id).map_err(|err| {
                    format!("failed to set IPv6 outgoing multicast interface to {scope_id}: {err}")
                })?;
            }
            scope_id
        }
        None => 0,
    };

    Ok((
        sender,
        SocketAddr::V6(SocketAddrV6::new(group, dst_port, 0, scope_id)),
    ))
}

fn send_once(sender: &UdpSocket, destination: SocketAddr, payload: &[u8]) -> Result<(), String> {
    let sent = sender
        .send_to(payload, destination)
        .map_err(|err| format!("failed to send packet: {err}"))?;

    println!("sent {sent} bytes to {destination}");
    Ok(())
}

fn parse_ip(name: &str, value: &str) -> Result<IpAddr, String> {
    value
        .parse::<IpAddr>()
        .map_err(|err| format!("invalid {name} '{value}': {err}"))
}

fn parse_interface_arg(group: IpAddr, value: &str) -> Result<InterfaceArg, String> {
    if let Ok(interface) = value.parse::<IpAddr>() {
        return Ok(InterfaceArg::Ip(interface));
    }

    if group.is_ipv6() {
        return value
            .parse::<u32>()
            .map(InterfaceArg::V6Index)
            .map_err(|err| {
                format!(
                    "invalid interface '{value}': expected an IPv6 address or interface index: {err}"
                )
            });
    }

    Err(format!(
        "invalid interface '{value}': expected an IPv4 address"
    ))
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

fn interface_string(interface: Option<InterfaceArg>) -> String {
    match interface {
        Some(InterfaceArg::Ip(interface)) => interface.to_string(),
        Some(InterfaceArg::V6Index(index)) => format!("ifindex:{index}"),
        None => "default".to_string(),
    }
}

fn set_outgoing_interface_v4(sender: &UdpSocket, interface: InterfaceArg) -> Result<(), String> {
    let interface = match interface {
        InterfaceArg::Ip(IpAddr::V4(interface)) => interface,
        InterfaceArg::Ip(IpAddr::V6(interface)) => {
            return Err(format!(
                "IPv6 interface address {interface} cannot be used with an IPv4 multicast group"
            ));
        }
        InterfaceArg::V6Index(index) => {
            return Err(format!(
                "IPv6 interface index {index} cannot be used with an IPv4 multicast group"
            ));
        }
    };

    let socket = SockRef::from(sender);
    socket
        .set_multicast_if_v4(&interface)
        .map_err(|err| format!("failed to set outgoing multicast interface to {interface}: {err}"))
}

fn resolve_ipv6_interface_arg(interface: InterfaceArg) -> Result<u32, String> {
    match interface {
        InterfaceArg::Ip(IpAddr::V6(interface)) if interface.is_unspecified() => Ok(0),
        InterfaceArg::Ip(IpAddr::V6(interface)) => resolve_ipv6_interface_index(interface),
        InterfaceArg::V6Index(index) => Ok(index),
        InterfaceArg::Ip(IpAddr::V4(interface)) => Err(format!(
            "IPv4 interface address {interface} cannot be used with an IPv6 multicast group"
        )),
    }
}

#[cfg(unix)]
fn resolve_ipv6_interface_index(interface: Ipv6Addr) -> Result<u32, String> {
    unsafe {
        let mut ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifaddrs) != 0 {
            return Err(format!(
                "failed to enumerate IPv6 interfaces: {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut cursor = ifaddrs;
        let mut matched_index = None;

        while !cursor.is_null() {
            let addr = (*cursor).ifa_addr;

            if !addr.is_null() && (*addr).sa_family as libc::c_int == libc::AF_INET6 {
                let sockaddr = &*(addr as *const libc::sockaddr_in6);
                if Ipv6Addr::from(sockaddr.sin6_addr.s6_addr) == interface {
                    let index = libc::if_nametoindex((*cursor).ifa_name);
                    if index != 0 {
                        matched_index = Some(index);
                        break;
                    }
                }
            }

            cursor = (*cursor).ifa_next;
        }

        libc::freeifaddrs(ifaddrs);

        matched_index.ok_or_else(|| {
            format!("failed to resolve IPv6 interface address {interface} to an interface index")
        })
    }
}

#[cfg(windows)]
fn resolve_ipv6_interface_index(interface: Ipv6Addr) -> Result<u32, String> {
    const INITIAL_BUFFER_SIZE: usize = 15_000;

    let mut buf_len = INITIAL_BUFFER_SIZE as u32;

    loop {
        let mut buffer = vec![0u8; buf_len as usize];
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                0,
                std::ptr::null(),
                buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(),
                &mut buf_len,
            )
        };

        if result == ERROR_BUFFER_OVERFLOW {
            continue;
        }

        if result != NO_ERROR {
            return Err(format!("GetAdaptersAddresses failed with status {result}"));
        }

        let mut adapter = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();

        unsafe {
            while !adapter.is_null() {
                let mut unicast = (*adapter).FirstUnicastAddress;

                while !unicast.is_null() {
                    let socket_address = (*unicast).Address;

                    if !socket_address.lpSockaddr.is_null()
                        && (*socket_address.lpSockaddr).sa_family == AF_INET6
                    {
                        let sockaddr = &*(socket_address.lpSockaddr as *const SOCKADDR_IN6);
                        let candidate = Ipv6Addr::from(sockaddr.sin6_addr.u.Byte);
                        if candidate == interface {
                            return Ok((*adapter).Ipv6IfIndex);
                        }
                    }

                    unicast = (*unicast).Next;
                }

                adapter = (*adapter).Next;
            }
        }

        return Err(format!(
            "failed to resolve IPv6 interface address {interface} to an interface index"
        ));
    }
}

#[cfg(not(any(unix, windows)))]
fn resolve_ipv6_interface_index(interface: Ipv6Addr) -> Result<u32, String> {
    Err(format!(
        "IPv6 interface resolution is not implemented on this platform for {interface}"
    ))
}

fn print_usage(program: &str) {
    eprintln!("Usage:");
    eprintln!("  {program} <group> <dst_port> <message> [interval_ms] [interface]");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  {program} 239.1.2.3 5000 hello");
    eprintln!("  {program} 239.1.2.3 5000 hello 1000");
    eprintln!("  {program} 232.1.2.3 5000 hello 1000 192.168.1.10");
    eprintln!("  {program} ff01::1234 5000 hello 1000 ::1");
    eprintln!("  {program} ff01::1234 5000 hello 1000 1");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_interface_arg_accepts_ipv4_address_for_ipv4_group() {
        let parsed = parse_interface_arg(IpAddr::V4(Ipv4Addr::LOCALHOST), "192.168.1.10").unwrap();
        assert_eq!(
            parsed,
            InterfaceArg::Ip(IpAddr::V4("192.168.1.10".parse().unwrap()))
        );
    }

    #[test]
    fn parse_interface_arg_accepts_ipv6_address_for_ipv6_group() {
        let parsed = parse_interface_arg(IpAddr::V6(Ipv6Addr::LOCALHOST), "::1").unwrap();
        assert_eq!(parsed, InterfaceArg::Ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn parse_interface_arg_accepts_ipv6_index_for_ipv6_group() {
        let parsed = parse_interface_arg(IpAddr::V6(Ipv6Addr::LOCALHOST), "7").unwrap();
        assert_eq!(parsed, InterfaceArg::V6Index(7));
    }

    #[test]
    fn parse_interface_arg_rejects_index_for_ipv4_group() {
        let err = parse_interface_arg(IpAddr::V4(Ipv4Addr::LOCALHOST), "7").unwrap_err();
        assert!(err.contains("expected an IPv4 address"));
    }
}
