use crate::error::McrxError;
use crate::raw::{RawPacket, RawSubscriptionConfig};
use crate::subscription::SubscriptionId;
use socket2::Socket;

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    windows,
    feature = "raw-shared-capture"
))]
use crate::config::SubscriptionAddressFamily;
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
use crate::packet::ReceiveMetadata;
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    windows,
    feature = "raw-shared-capture"
))]
use bytes::Bytes;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use socket2::{Domain, Protocol, SockAddr, Type};
#[cfg(target_os = "macos")]
use std::ffi::{CStr, CString};
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::io::ErrorKind;
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    windows,
    test,
    feature = "raw-shared-capture"
))]
use std::net::IpAddr;
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
use std::net::{Ipv4Addr, Ipv6Addr};
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::net::{SocketAddrV4, SocketAddrV6};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, RawSocket};
#[cfg(target_os = "macos")]
use std::sync::Mutex;
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::{
    IPPROTO_IP, IPPROTO_UDP, RCVALL_ON, SIO_RCVALL, SIO_RCVALL_MCAST, SOCKET, SOCKET_ERROR,
    WSAGetLastError, WSAIoctl,
};

pub(crate) struct RawReceiveSocket {
    receive_socket: Socket,
    membership_socket: Option<Socket>,
    #[cfg(windows)]
    windows_udp_receive_socket: Socket,
    #[cfg(target_os = "macos")]
    apple_bpf_state: Mutex<AppleBpfState>,
    #[cfg(target_os = "macos")]
    apple_bpf_datalink: u32,
    #[cfg(target_os = "macos")]
    apple_interface_index: Option<u32>,
}

#[cfg(target_os = "macos")]
struct AppleBpfState {
    buffer: Vec<u8>,
    filled_len: usize,
    next_offset: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[repr(C)]
#[derive(Clone, Copy)]
struct ClassicBpfInstruction {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct AppleBpfProgram {
    len: u32,
    instructions: *mut ClassicBpfInstruction,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const BPF_LD: u16 = 0x00;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BPF_W: u16 = 0x00;
#[cfg(target_os = "macos")]
const BPF_H: u16 = 0x08;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BPF_B: u16 = 0x10;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BPF_ABS: u16 = 0x20;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BPF_ALU: u16 = 0x04;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BPF_AND: u16 = 0x50;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BPF_JMP: u16 = 0x05;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BPF_JEQ: u16 = 0x10;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BPF_K: u16 = 0x00;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BPF_RET: u16 = 0x06;

impl std::fmt::Debug for RawReceiveSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawReceiveSocket").finish_non_exhaustive()
    }
}

impl RawReceiveSocket {
    pub(crate) fn socket(&self) -> &Socket {
        &self.receive_socket
    }
}

/// Normalized capture-socket identity for shared raw receive.
///
/// Linux packet sockets are shared only when both the IP family and selected
/// interface index match. Index zero uses a wildcard packet-socket bind while
/// the companion membership socket uses the platform-selected default.
#[cfg(feature = "raw-shared-capture")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RawSharedCaptureKey {
    pub(crate) family: SubscriptionAddressFamily,
    pub(crate) interface_index: u32,
}

/// A parsed IP datagram read once from a shared capture socket.
#[cfg(feature = "raw-shared-capture")]
#[derive(Debug, Clone)]
pub(crate) struct RawCapturedDatagram {
    pub(crate) datagram: Bytes,
    pub(crate) source_ip: IpAddr,
    pub(crate) group: IpAddr,
    pub(crate) ip_protocol: u8,
    pub(crate) ingress_interface_index: Option<u32>,
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedIpDatagram {
    source_ip: IpAddr,
    destination_ip: IpAddr,
    protocol: u8,
    datagram_len: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
const MAX_RAW_FILTERED_READS_PER_RECV: usize = 256;
#[cfg(any(target_os = "linux", windows))]
const MAX_IP_DATAGRAM_SIZE: usize = 40 + u16::MAX as usize;

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn raw_filter_budget_exhausted(filtered_reads: &mut usize) -> bool {
    // Raw capture sockets can see unrelated interface traffic. Keep one
    // non-blocking receive call bounded even when the interface is noisy.
    *filtered_reads += 1;
    *filtered_reads >= MAX_RAW_FILTERED_READS_PER_RECV
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn bpf_statement(code: u16, value: u32) -> ClassicBpfInstruction {
    ClassicBpfInstruction {
        code,
        jt: 0,
        jf: 0,
        k: value,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn bpf_ip_filter(config: &RawSubscriptionConfig, ip_offset: u32) -> Vec<ClassicBpfInstruction> {
    fn add_comparison(
        instructions: &mut Vec<ClassicBpfInstruction>,
        rejects: &mut Vec<usize>,
        offset: u32,
        expected: u32,
    ) {
        instructions.push(bpf_statement(BPF_LD | BPF_W | BPF_ABS, offset));
        rejects.push(instructions.len());
        instructions.push(bpf_statement(BPF_JMP | BPF_JEQ | BPF_K, expected));
    }

    let mut instructions = vec![
        bpf_statement(BPF_LD | BPF_B | BPF_ABS, ip_offset),
        bpf_statement(BPF_ALU | BPF_AND | BPF_K, 0xf0),
    ];
    let mut rejects = vec![instructions.len()];
    instructions.push(bpf_statement(
        BPF_JMP | BPF_JEQ | BPF_K,
        if config.is_ipv4() { 0x40 } else { 0x60 },
    ));

    let (source_offset, destination_offset) = if config.is_ipv4() {
        (ip_offset + 12, ip_offset + 16)
    } else {
        (ip_offset + 8, ip_offset + 24)
    };
    let destination = match config.group {
        IpAddr::V4(address) => address.octets().to_vec(),
        IpAddr::V6(address) => address.octets().to_vec(),
    };

    for (word, bytes) in destination.chunks_exact(4).enumerate() {
        add_comparison(
            &mut instructions,
            &mut rejects,
            destination_offset + word as u32 * 4,
            u32::from_be_bytes(bytes.try_into().expect("four-byte address word")),
        );
    }

    if let crate::SourceFilter::Source(source) = config.source {
        let source = match source {
            IpAddr::V4(address) => address.octets().to_vec(),
            IpAddr::V6(address) => address.octets().to_vec(),
        };
        for (word, bytes) in source.chunks_exact(4).enumerate() {
            add_comparison(
                &mut instructions,
                &mut rejects,
                source_offset + word as u32 * 4,
                u32::from_be_bytes(bytes.try_into().expect("four-byte address word")),
            );
        }
    }

    instructions.push(bpf_statement(BPF_RET | BPF_K, u32::MAX));
    let reject_index = instructions.len();
    instructions.push(bpf_statement(BPF_RET | BPF_K, 0));

    for comparison_index in rejects {
        instructions[comparison_index].jf = (reject_index - comparison_index - 1) as u8;
    }

    instructions
}

#[cfg(all(target_os = "linux", feature = "raw-shared-capture"))]
fn bpf_ip_multicast_family_filter(
    family: SubscriptionAddressFamily,
    ip_offset: u32,
) -> Vec<ClassicBpfInstruction> {
    let (version, destination_offset, destination_mask, destination_value) = match family {
        SubscriptionAddressFamily::Ipv4 => (0x40, ip_offset + 16, Some(0xf0), 0xe0),
        SubscriptionAddressFamily::Ipv6 => (0x60, ip_offset + 24, None, 0xff),
    };

    let mut instructions = vec![
        bpf_statement(BPF_LD | BPF_B | BPF_ABS, ip_offset),
        bpf_statement(BPF_ALU | BPF_AND | BPF_K, 0xf0),
    ];
    let mut rejects = vec![instructions.len()];
    instructions.push(bpf_statement(BPF_JMP | BPF_JEQ | BPF_K, version));
    instructions.push(bpf_statement(BPF_LD | BPF_B | BPF_ABS, destination_offset));
    if let Some(mask) = destination_mask {
        instructions.push(bpf_statement(BPF_ALU | BPF_AND | BPF_K, mask));
    }
    rejects.push(instructions.len());
    instructions.push(bpf_statement(BPF_JMP | BPF_JEQ | BPF_K, destination_value));
    instructions.push(bpf_statement(BPF_RET | BPF_K, u32::MAX));
    let reject_index = instructions.len();
    instructions.push(bpf_statement(BPF_RET | BPF_K, 0));

    for comparison_index in rejects {
        instructions[comparison_index].jf = (reject_index - comparison_index - 1) as u8;
    }

    instructions
}

#[cfg(target_os = "linux")]
pub(crate) fn open_raw_socket(
    config: &RawSubscriptionConfig,
) -> Result<RawReceiveSocket, McrxError> {
    config.validate()?;

    let packet_socket = open_linux_packet_socket(config)?;
    let membership_socket = open_membership_socket(config.family())?;

    Ok(RawReceiveSocket {
        receive_socket: packet_socket,
        membership_socket: Some(membership_socket),
    })
}

#[cfg(all(target_os = "linux", feature = "raw-shared-capture"))]
pub(crate) fn shared_raw_capture_key(
    config: &RawSubscriptionConfig,
) -> Result<RawSharedCaptureKey, McrxError> {
    config.validate()?;

    Ok(RawSharedCaptureKey {
        family: config.family(),
        interface_index: resolve_linux_packet_interface_index(config)?,
    })
}

#[cfg(all(target_os = "linux", feature = "raw-shared-capture"))]
pub(crate) fn open_shared_raw_socket(
    key: RawSharedCaptureKey,
) -> Result<RawReceiveSocket, McrxError> {
    let packet_socket = open_linux_packet_socket_with_filter(
        key.interface_index,
        bpf_ip_multicast_family_filter(key.family, 0),
    )?;
    let membership_socket = open_membership_socket(key.family)?;

    Ok(RawReceiveSocket {
        receive_socket: packet_socket,
        membership_socket: Some(membership_socket),
    })
}

#[cfg(all(target_os = "linux", feature = "raw-shared-capture"))]
pub(crate) fn recv_shared_raw_datagram(
    socket: &RawReceiveSocket,
    key: RawSharedCaptureKey,
) -> Result<Option<RawCapturedDatagram>, McrxError> {
    let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); MAX_IP_DATAGRAM_SIZE];
    let mut filtered_reads = 0usize;

    loop {
        let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        let mut addr_len = std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;

        let len = unsafe {
            libc::recvfrom(
                socket.socket().as_raw_fd(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                0,
                (&mut addr as *mut libc::sockaddr_ll).cast(),
                &mut addr_len,
            )
        };

        if len == -1 {
            let err = std::io::Error::last_os_error();
            if err.kind() == ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(McrxError::ReceiveFailed(err));
        }

        let len = len as usize;
        // SAFETY: recvfrom initialized exactly the returned prefix of buf.
        let datagram = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len) };
        let Some(parsed) = parse_ip_datagram(datagram) else {
            if raw_filter_budget_exhausted(&mut filtered_reads) {
                return Ok(None);
            }
            continue;
        };

        if !matches!(
            (key.family, parsed.destination_ip),
            (SubscriptionAddressFamily::Ipv4, IpAddr::V4(_))
                | (SubscriptionAddressFamily::Ipv6, IpAddr::V6(_))
        ) {
            if raw_filter_budget_exhausted(&mut filtered_reads) {
                return Ok(None);
            }
            continue;
        }

        return Ok(Some(RawCapturedDatagram {
            datagram: Bytes::copy_from_slice(&datagram[..parsed.datagram_len]),
            source_ip: parsed.source_ip,
            group: parsed.destination_ip,
            ip_protocol: parsed.protocol,
            ingress_interface_index: (addr.sll_ifindex > 0).then_some(addr.sll_ifindex as u32),
        }));
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn open_raw_socket(
    config: &RawSubscriptionConfig,
) -> Result<RawReceiveSocket, McrxError> {
    config.validate()?;

    let interface_index = resolve_packet_interface_index(config)?.ok_or_else(|| {
        McrxError::RawPacketReceiveUnsupported(
            "macOS raw multicast receive requires an explicit interface address or index"
                .to_string(),
        )
    })?;

    let bpf_socket = open_apple_bpf_socket(interface_index, config)?;
    let membership_socket = open_membership_socket(config.family())?;

    Ok(RawReceiveSocket {
        receive_socket: bpf_socket.socket,
        membership_socket: Some(membership_socket),
        apple_bpf_state: Mutex::new(AppleBpfState {
            buffer: vec![0u8; bpf_socket.buffer_len],
            filled_len: 0,
            next_offset: 0,
        }),
        apple_bpf_datalink: bpf_socket.datalink,
        apple_interface_index: Some(interface_index),
    })
}

#[cfg(windows)]
pub(crate) fn open_raw_socket(
    config: &RawSubscriptionConfig,
) -> Result<RawReceiveSocket, McrxError> {
    config.validate()?;

    if config.is_ipv6() {
        return Err(McrxError::RawPacketReceiveUnsupported(
            "Windows raw multicast receive currently supports IPv4 only".to_string(),
        ));
    }

    let interface = match config.interface {
        Some(IpAddr::V4(interface)) if !interface.is_unspecified() => interface,
        _ => {
            return Err(McrxError::RawPacketReceiveUnsupported(
                "Windows raw IPv4 receive requires an explicit local IPv4 interface address"
                    .to_string(),
            ));
        }
    };

    let raw_socket = open_windows_raw_socket(interface)?;
    let udp_raw_socket = open_windows_udp_raw_socket(interface)?;
    let membership_socket = open_membership_socket(config.family())?;

    Ok(RawReceiveSocket {
        receive_socket: raw_socket,
        membership_socket: Some(membership_socket),
        windows_udp_receive_socket: udp_raw_socket,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn open_raw_socket(
    _config: &RawSubscriptionConfig,
) -> Result<RawReceiveSocket, McrxError> {
    Err(McrxError::RawPacketReceiveUnsupported(
        "raw multicast receive is currently implemented on Linux, macOS, and Windows".to_string(),
    ))
}

#[cfg(all(feature = "raw-shared-capture", not(target_os = "linux")))]
pub(crate) fn shared_raw_capture_key(
    _config: &RawSubscriptionConfig,
) -> Result<RawSharedCaptureKey, McrxError> {
    Err(McrxError::RawPacketReceiveUnsupported(
        "shared raw capture is currently implemented on Linux only".to_string(),
    ))
}

#[cfg(all(feature = "raw-shared-capture", not(target_os = "linux")))]
pub(crate) fn open_shared_raw_socket(
    _key: RawSharedCaptureKey,
) -> Result<RawReceiveSocket, McrxError> {
    Err(McrxError::RawPacketReceiveUnsupported(
        "shared raw capture is currently implemented on Linux only".to_string(),
    ))
}

#[cfg(all(feature = "raw-shared-capture", not(target_os = "linux")))]
pub(crate) fn recv_shared_raw_datagram(
    _socket: &RawReceiveSocket,
    _key: RawSharedCaptureKey,
) -> Result<Option<RawCapturedDatagram>, McrxError> {
    Err(McrxError::RawPacketReceiveUnsupported(
        "shared raw capture is currently implemented on Linux only".to_string(),
    ))
}

pub(crate) fn join_raw_multicast_group(
    socket: &RawReceiveSocket,
    config: &RawSubscriptionConfig,
) -> Result<(), McrxError> {
    let membership_socket = socket.membership_socket.as_ref().ok_or_else(|| {
        McrxError::RawPacketReceiveUnsupported(
            "raw multicast membership sockets are not available on this platform".to_string(),
        )
    })?;

    let compat_config = config.membership_compat_config();
    super::join_multicast_group(membership_socket, &compat_config)?;

    #[cfg(windows)]
    if let Err(err) = join_windows_raw_receive_sockets(socket, &compat_config) {
        let _ = super::leave_multicast_group(membership_socket, &compat_config);
        return Err(err);
    }

    Ok(())
}

pub(crate) fn leave_raw_multicast_group(
    socket: &RawReceiveSocket,
    config: &RawSubscriptionConfig,
) -> Result<(), McrxError> {
    let membership_socket = socket.membership_socket.as_ref().ok_or_else(|| {
        McrxError::RawPacketReceiveUnsupported(
            "raw multicast membership sockets are not available on this platform".to_string(),
        )
    })?;

    let compat_config = config.membership_compat_config();
    super::leave_multicast_group(membership_socket, &compat_config)?;

    #[cfg(windows)]
    leave_windows_raw_receive_sockets(socket, &compat_config)?;

    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn recv_raw_packet(
    socket: &RawReceiveSocket,
    subscription_id: SubscriptionId,
    config: &RawSubscriptionConfig,
) -> Result<Option<RawPacket>, McrxError> {
    let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); MAX_IP_DATAGRAM_SIZE];
    let mut filtered_reads = 0usize;

    loop {
        let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        let mut addr_len = std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;

        let len = unsafe {
            libc::recvfrom(
                socket.socket().as_raw_fd(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                0,
                (&mut addr as *mut libc::sockaddr_ll).cast(),
                &mut addr_len,
            )
        };

        if len == -1 {
            let err = std::io::Error::last_os_error();
            if err.kind() == ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(McrxError::ReceiveFailed(err));
        }

        let len = len as usize;
        // SAFETY: `recvfrom` initialized exactly the first `len` bytes of `buf`.
        let datagram = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, len) };

        let Some(parsed) = parse_ip_datagram(datagram) else {
            if raw_filter_budget_exhausted(&mut filtered_reads) {
                return Ok(None);
            }
            continue;
        };

        if !packet_matches_config(parsed, config) {
            if raw_filter_budget_exhausted(&mut filtered_reads) {
                return Ok(None);
            }
            continue;
        }

        let mut metadata = ReceiveMetadata::empty();
        metadata.configured_interface = config.interface;
        metadata.configured_interface_index = config.interface_index;
        metadata.destination_local_ip = Some(parsed.destination_ip);
        if addr.sll_ifindex > 0 {
            metadata.ingress_interface_index = Some(addr.sll_ifindex as u32);
        }

        return Ok(Some(raw_packet_from_parts(
            subscription_id,
            datagram,
            parsed,
            metadata,
        )));
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn recv_raw_packet(
    socket: &RawReceiveSocket,
    subscription_id: SubscriptionId,
    config: &RawSubscriptionConfig,
) -> Result<Option<RawPacket>, McrxError> {
    let mut filtered_reads = 0usize;

    loop {
        let mut state = socket.apple_bpf_state.lock().map_err(|_| {
            McrxError::ReceiveFailed(std::io::Error::other("macOS BPF buffer mutex is poisoned"))
        })?;

        if state.next_offset < state.filled_len {
            let start_offset = state.next_offset;
            let (packet, next_offset) = next_matching_apple_bpf_packet(
                &state.buffer[..state.filled_len],
                start_offset,
                socket.apple_bpf_datalink,
                socket.apple_interface_index,
                subscription_id,
                config,
            )?;
            state.next_offset = next_offset;

            if let Some(packet) = packet {
                return Ok(Some(packet));
            }

            state.filled_len = 0;
            state.next_offset = 0;
            drop(state);

            if raw_filter_budget_exhausted(&mut filtered_reads) {
                return Ok(None);
            }
            continue;
        }

        let len = unsafe {
            libc::read(
                socket.socket().as_raw_fd(),
                state.buffer.as_mut_ptr().cast(),
                state.buffer.len(),
            )
        };

        if len == -1 {
            let err = std::io::Error::last_os_error();
            if err.kind() == ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(McrxError::ReceiveFailed(err));
        }

        state.filled_len = len as usize;
        state.next_offset = 0;
    }
}

#[cfg(windows)]
pub(crate) fn recv_raw_packet(
    socket: &RawReceiveSocket,
    subscription_id: SubscriptionId,
    config: &RawSubscriptionConfig,
) -> Result<Option<RawPacket>, McrxError> {
    if let Some(packet) = recv_raw_packet_from_windows_socket(
        &socket.receive_socket,
        subscription_id,
        config,
        Some(IPPROTO_UDP as u8),
    )? {
        return Ok(Some(packet));
    }

    recv_raw_packet_from_windows_socket(
        &socket.windows_udp_receive_socket,
        subscription_id,
        config,
        None,
    )
}

#[cfg(windows)]
fn recv_raw_packet_from_windows_socket(
    receive_socket: &Socket,
    subscription_id: SubscriptionId,
    config: &RawSubscriptionConfig,
    excluded_protocol: Option<u8>,
) -> Result<Option<RawPacket>, McrxError> {
    let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); MAX_IP_DATAGRAM_SIZE];
    let mut filtered_reads = 0usize;

    loop {
        let len = unsafe {
            windows_sys::Win32::Networking::WinSock::recv(
                windows_raw_socket(receive_socket),
                buf.as_mut_ptr().cast(),
                buf.len() as i32,
                0,
            )
        };

        if len == SOCKET_ERROR {
            let err = last_wsa_error();
            if err.kind() == ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(McrxError::ReceiveFailed(err));
        }

        let len = len as usize;
        // SAFETY: `recv` initialized exactly the first `len` bytes of `buf`.
        let datagram = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, len) };

        let Some(parsed) = parse_ip_datagram(datagram) else {
            if raw_filter_budget_exhausted(&mut filtered_reads) {
                return Ok(None);
            }
            continue;
        };

        if excluded_protocol == Some(parsed.protocol) {
            if raw_filter_budget_exhausted(&mut filtered_reads) {
                return Ok(None);
            }
            continue;
        }

        if !packet_matches_config(parsed, config) {
            if raw_filter_budget_exhausted(&mut filtered_reads) {
                return Ok(None);
            }
            continue;
        }

        let mut metadata = ReceiveMetadata::empty();
        metadata.configured_interface = config.interface;
        metadata.configured_interface_index = config.interface_index;
        metadata.destination_local_ip = Some(parsed.destination_ip);

        return Ok(Some(raw_packet_from_parts(
            subscription_id,
            datagram,
            parsed,
            metadata,
        )));
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn recv_raw_packet(
    _socket: &RawReceiveSocket,
    _subscription_id: SubscriptionId,
    _config: &RawSubscriptionConfig,
) -> Result<Option<RawPacket>, McrxError> {
    Err(McrxError::RawPacketReceiveUnsupported(
        "raw multicast receive is currently implemented on Linux, macOS, and Windows".to_string(),
    ))
}

#[cfg(target_os = "linux")]
fn open_linux_packet_socket(config: &RawSubscriptionConfig) -> Result<Socket, McrxError> {
    open_linux_packet_socket_with_filter(
        resolve_linux_packet_interface_index(config)?,
        bpf_ip_filter(config, 0),
    )
}

#[cfg(target_os = "linux")]
fn open_linux_packet_socket_with_filter(
    interface_index: u32,
    instructions: Vec<ClassicBpfInstruction>,
) -> Result<Socket, McrxError> {
    let protocol = linux_packet_socket_protocol();
    let raw_fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_DGRAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            protocol as i32,
        )
    };

    if raw_fd == -1 {
        return Err(McrxError::RawSocketCreateFailed(
            std::io::Error::last_os_error(),
        ));
    }

    let socket = unsafe { Socket::from_raw_fd(raw_fd) };
    attach_linux_filter(&socket, instructions)?;

    let bind_addr = libc::sockaddr_ll {
        sll_family: libc::AF_PACKET as u16,
        sll_protocol: protocol,
        sll_ifindex: interface_index as i32,
        sll_hatype: 0,
        sll_pkttype: 0,
        sll_halen: 0,
        sll_addr: [0; 8],
    };

    let result = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&bind_addr as *const libc::sockaddr_ll).cast(),
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };

    if result == -1 {
        return Err(McrxError::RawSocketBindFailed(
            std::io::Error::last_os_error(),
        ));
    }

    Ok(socket)
}

#[cfg(target_os = "linux")]
fn linux_packet_socket_protocol() -> u16 {
    (libc::ETH_P_ALL as u16).to_be()
}

#[cfg(target_os = "linux")]
fn attach_linux_filter(
    socket: &Socket,
    mut instructions: Vec<ClassicBpfInstruction>,
) -> Result<(), McrxError> {
    const _: () = assert!(
        std::mem::size_of::<ClassicBpfInstruction>() == std::mem::size_of::<libc::sock_filter>()
    );
    const _: () = assert!(
        std::mem::align_of::<ClassicBpfInstruction>() == std::mem::align_of::<libc::sock_filter>()
    );

    let program = libc::sock_fprog {
        len: instructions.len() as u16,
        filter: instructions.as_mut_ptr().cast::<libc::sock_filter>(),
    };

    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_ATTACH_FILTER,
            (&program as *const libc::sock_fprog).cast(),
            std::mem::size_of_val(&program) as libc::socklen_t,
        )
    };

    if result == -1 {
        Err(McrxError::SocketOptionFailed(
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
struct AppleBpfSocket {
    socket: Socket,
    buffer_len: usize,
    datalink: u32,
}

#[cfg(target_os = "macos")]
fn open_apple_bpf_socket(
    interface_index: u32,
    config: &RawSubscriptionConfig,
) -> Result<AppleBpfSocket, McrxError> {
    let interface_name = interface_name_from_index(interface_index)?;
    let socket = open_available_bpf_device()?;

    set_fd_nonblocking(socket.as_raw_fd())?;
    bind_bpf_to_interface(&socket, &interface_name)?;
    set_bpf_u32(&socket, libc::BIOCIMMEDIATE, 1)?;
    set_bpf_u32(&socket, libc::BIOCSSEESENT, 1)?;

    let buffer_len = get_bpf_u32(&socket, libc::BIOCGBLEN)? as usize;
    let datalink = get_bpf_u32(&socket, libc::BIOCGDLT)?;
    attach_apple_subscription_filter(&socket, datalink, config)?;

    Ok(AppleBpfSocket {
        socket,
        buffer_len,
        datalink,
    })
}

#[cfg(target_os = "macos")]
fn attach_apple_subscription_filter(
    socket: &Socket,
    datalink: u32,
    config: &RawSubscriptionConfig,
) -> Result<(), McrxError> {
    let mut instructions = apple_subscription_filter(datalink, config)?;
    let mut program = AppleBpfProgram {
        len: instructions.len() as u32,
        instructions: instructions.as_mut_ptr(),
    };
    let result = unsafe {
        libc::ioctl(
            socket.as_raw_fd(),
            libc::BIOCSETF,
            (&mut program as *mut AppleBpfProgram).cast::<libc::c_void>(),
        )
    };

    if result == -1 {
        Err(McrxError::SocketOptionFailed(
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn apple_subscription_filter(
    datalink: u32,
    config: &RawSubscriptionConfig,
) -> Result<Vec<ClassicBpfInstruction>, McrxError> {
    match datalink {
        libc::DLT_RAW => Ok(bpf_ip_filter(config, 0)),
        libc::DLT_NULL | libc::DLT_LOOP => Ok(bpf_ip_filter(config, 4)),
        libc::DLT_EN10MB => {
            let expected_ethertype = if config.is_ipv4() { 0x0800 } else { 0x86dd };
            let mut instructions = vec![
                bpf_statement(BPF_LD | BPF_H | BPF_ABS, 12),
                ClassicBpfInstruction {
                    code: BPF_JMP | BPF_JEQ | BPF_K,
                    jt: 7,
                    jf: 0,
                    k: expected_ethertype,
                },
                ClassicBpfInstruction {
                    code: BPF_JMP | BPF_JEQ | BPF_K,
                    jt: 0,
                    jf: 1,
                    k: 0x8100,
                },
                bpf_statement(BPF_RET | BPF_K, u32::MAX),
                ClassicBpfInstruction {
                    code: BPF_JMP | BPF_JEQ | BPF_K,
                    jt: 0,
                    jf: 1,
                    k: 0x88a8,
                },
                bpf_statement(BPF_RET | BPF_K, u32::MAX),
                ClassicBpfInstruction {
                    code: BPF_JMP | BPF_JEQ | BPF_K,
                    jt: 0,
                    jf: 1,
                    k: 0x9100,
                },
                bpf_statement(BPF_RET | BPF_K, u32::MAX),
                bpf_statement(BPF_RET | BPF_K, 0),
            ];
            instructions.extend(bpf_ip_filter(config, 14));
            Ok(instructions)
        }
        other => Err(McrxError::RawPacketReceiveUnsupported(format!(
            "macOS BPF datalink type {other} is not supported yet"
        ))),
    }
}

#[cfg(target_os = "macos")]
fn open_available_bpf_device() -> Result<Socket, McrxError> {
    for index in 0..256 {
        let path = CString::new(format!("/dev/bpf{index}")).expect("BPF path has no nul bytes");
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };

        if fd != -1 {
            return Ok(unsafe { Socket::from_raw_fd(fd) });
        }

        let err = std::io::Error::last_os_error();
        if err.kind() == ErrorKind::PermissionDenied {
            return Err(McrxError::RawSocketCreateFailed(err));
        }

        if err.raw_os_error() != Some(libc::EBUSY) && err.kind() != ErrorKind::NotFound {
            return Err(McrxError::RawSocketCreateFailed(err));
        }
    }

    Err(McrxError::RawSocketCreateFailed(std::io::Error::new(
        ErrorKind::NotFound,
        "no available /dev/bpf device found",
    )))
}

#[cfg(target_os = "macos")]
fn bind_bpf_to_interface(socket: &Socket, interface_name: &CStr) -> Result<(), McrxError> {
    let mut request = unsafe { std::mem::zeroed::<libc::ifreq>() };
    let bytes = interface_name.to_bytes_with_nul();

    if bytes.len() > request.ifr_name.len() {
        return Err(McrxError::InterfaceDiscoveryFailed(format!(
            "interface name {} is too long for BPF",
            interface_name.to_string_lossy()
        )));
    }

    for (dst, src) in request.ifr_name.iter_mut().zip(bytes.iter().copied()) {
        *dst = src as libc::c_char;
    }

    let result = unsafe {
        libc::ioctl(
            socket.as_raw_fd(),
            libc::BIOCSETIF,
            (&mut request as *mut libc::ifreq).cast::<libc::c_void>(),
        )
    };

    if result == -1 {
        Err(McrxError::RawSocketBindFailed(
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn set_bpf_u32(socket: &Socket, request: libc::c_ulong, value: u32) -> Result<(), McrxError> {
    let mut value = value as libc::c_uint;
    let result = unsafe {
        libc::ioctl(
            socket.as_raw_fd(),
            request,
            (&mut value as *mut libc::c_uint).cast::<libc::c_void>(),
        )
    };

    if result == -1 {
        Err(McrxError::SocketOptionFailed(
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn get_bpf_u32(socket: &Socket, request: libc::c_ulong) -> Result<u32, McrxError> {
    let mut value = 0 as libc::c_uint;
    let result = unsafe {
        libc::ioctl(
            socket.as_raw_fd(),
            request,
            (&mut value as *mut libc::c_uint).cast::<libc::c_void>(),
        )
    };

    if result == -1 {
        Err(McrxError::SocketOptionFailed(
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(value)
    }
}

#[cfg(target_os = "macos")]
fn set_fd_nonblocking(fd: std::os::fd::RawFd) -> Result<(), McrxError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(McrxError::SocketOptionFailed(
            std::io::Error::last_os_error(),
        ));
    }

    let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if result == -1 {
        Err(McrxError::SocketOptionFailed(
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn interface_name_from_index(interface_index: u32) -> Result<CString, McrxError> {
    let mut name = [0 as libc::c_char; libc::IFNAMSIZ];
    let ptr = unsafe { libc::if_indextoname(interface_index, name.as_mut_ptr()) };

    if ptr.is_null() {
        return Err(McrxError::InterfaceDiscoveryFailed(format!(
            "failed to resolve interface index {interface_index} to an interface name"
        )));
    }

    Ok(unsafe { CStr::from_ptr(ptr) }.to_owned())
}

#[cfg(windows)]
fn open_windows_raw_socket(interface: Ipv4Addr) -> Result<Socket, McrxError> {
    let socket = Socket::new(Domain::IPV4, Type::RAW, Some(Protocol::from(IPPROTO_IP)))
        .map_err(McrxError::RawSocketCreateFailed)?;

    socket
        .bind(&SockAddr::from(SocketAddrV4::new(interface, 0)))
        .map_err(McrxError::RawSocketBindFailed)?;

    socket
        .set_nonblocking(true)
        .map_err(McrxError::SocketOptionFailed)?;

    enable_windows_raw_capture(&socket)?;
    Ok(socket)
}

#[cfg(windows)]
fn open_windows_udp_raw_socket(interface: Ipv4Addr) -> Result<Socket, McrxError> {
    let socket = Socket::new(Domain::IPV4, Type::RAW, Some(Protocol::from(IPPROTO_UDP)))
        .map_err(McrxError::RawSocketCreateFailed)?;

    socket
        .bind(&SockAddr::from(SocketAddrV4::new(interface, 0)))
        .map_err(McrxError::RawSocketBindFailed)?;

    socket
        .set_nonblocking(true)
        .map_err(McrxError::SocketOptionFailed)?;

    Ok(socket)
}

#[cfg(windows)]
fn join_windows_raw_receive_sockets(
    socket: &RawReceiveSocket,
    config: &crate::SubscriptionConfig,
) -> Result<(), McrxError> {
    if !matches!(config.family(), SubscriptionAddressFamily::Ipv4) {
        return Ok(());
    }

    super::join_multicast_group(&socket.receive_socket, config)?;
    if let Err(err) = super::join_multicast_group(&socket.windows_udp_receive_socket, config) {
        let _ = super::leave_multicast_group(&socket.receive_socket, config);
        return Err(err);
    }

    Ok(())
}

#[cfg(windows)]
fn leave_windows_raw_receive_sockets(
    socket: &RawReceiveSocket,
    config: &crate::SubscriptionConfig,
) -> Result<(), McrxError> {
    if !matches!(config.family(), SubscriptionAddressFamily::Ipv4) {
        return Ok(());
    }

    let primary = super::leave_multicast_group(&socket.receive_socket, config);
    let udp = super::leave_multicast_group(&socket.windows_udp_receive_socket, config);
    match (primary, udp) {
        (Err(err), _) | (Ok(()), Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(windows)]
fn enable_windows_raw_capture(socket: &Socket) -> Result<(), McrxError> {
    match windows_raw_capture_ioctl(socket, SIO_RCVALL_MCAST) {
        Ok(()) => Ok(()),
        Err(_) => windows_raw_capture_ioctl(socket, SIO_RCVALL),
    }
}

#[cfg(windows)]
fn windows_raw_capture_ioctl(socket: &Socket, code: u32) -> Result<(), McrxError> {
    let mode = RCVALL_ON;
    let mut bytes_returned = 0u32;
    let result = unsafe {
        WSAIoctl(
            windows_raw_socket(socket),
            code,
            (&mode as *const i32).cast(),
            std::mem::size_of_val(&mode) as u32,
            std::ptr::null_mut(),
            0,
            &mut bytes_returned,
            std::ptr::null_mut(),
            None,
        )
    };

    if result == SOCKET_ERROR {
        Err(McrxError::SocketIoctlFailed(last_wsa_error()))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn windows_raw_socket(socket: &Socket) -> SOCKET {
    socket.as_raw_socket() as RawSocket as SOCKET
}

#[cfg(windows)]
fn last_wsa_error() -> std::io::Error {
    std::io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn open_membership_socket(family: SubscriptionAddressFamily) -> Result<Socket, McrxError> {
    let socket = match family {
        SubscriptionAddressFamily::Ipv4 => {
            Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
                .map_err(McrxError::SocketCreateFailed)?
        }
        SubscriptionAddressFamily::Ipv6 => {
            let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
                .map_err(McrxError::SocketCreateFailed)?;
            socket
                .set_only_v6(true)
                .map_err(McrxError::SocketOptionFailed)?;
            socket
        }
    };

    socket
        .set_nonblocking(true)
        .map_err(McrxError::SocketOptionFailed)?;

    match family {
        SubscriptionAddressFamily::Ipv4 => socket
            .bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)))
            .map_err(McrxError::SocketBindFailed)?,
        SubscriptionAddressFamily::Ipv6 => socket
            .bind(&SockAddr::from(SocketAddrV6::new(
                Ipv6Addr::UNSPECIFIED,
                0,
                0,
                0,
            )))
            .map_err(McrxError::SocketBindFailed)?,
    }

    Ok(socket)
}

#[cfg(target_os = "linux")]
fn resolve_linux_packet_interface_index(config: &RawSubscriptionConfig) -> Result<u32, McrxError> {
    Ok(resolve_packet_interface_index(config)?.unwrap_or(0))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn resolve_packet_interface_index(
    config: &RawSubscriptionConfig,
) -> Result<Option<u32>, McrxError> {
    match (config.interface_index, config.interface) {
        (Some(interface_index), _) => Ok(Some(interface_index)),
        (None, Some(IpAddr::V6(interface))) if !interface.is_unspecified() => {
            super::resolve_ipv6_interface_index(interface).map(Some)
        }
        (None, Some(IpAddr::V4(interface))) if !interface.is_unspecified() => {
            resolve_ipv4_interface_index(interface).map(Some)
        }
        _ => Ok(None),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn resolve_ipv4_interface_index(interface: Ipv4Addr) -> Result<u32, McrxError> {
    unsafe {
        let mut ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifaddrs) != 0 {
            return Err(McrxError::InterfaceDiscoveryFailed(
                std::io::Error::last_os_error().to_string(),
            ));
        }

        let mut cursor = ifaddrs;
        let mut matched_index = None;

        while !cursor.is_null() {
            let addr = (*cursor).ifa_addr;

            if !addr.is_null()
                && !(*cursor).ifa_name.is_null()
                && (*addr).sa_family as libc::c_int == libc::AF_INET
            {
                let sockaddr = &*(addr as *const libc::sockaddr_in);
                let candidate = Ipv4Addr::from(u32::from_be(sockaddr.sin_addr.s_addr));
                if candidate == interface {
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
            McrxError::InterfaceDiscoveryFailed(format!(
                "failed to resolve IPv4 interface address {interface} to an interface index"
            ))
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn packet_matches_config(parsed: ParsedIpDatagram, config: &RawSubscriptionConfig) -> bool {
    if !matches!(
        (config.family(), parsed.destination_ip),
        (SubscriptionAddressFamily::Ipv4, IpAddr::V4(_))
            | (SubscriptionAddressFamily::Ipv6, IpAddr::V6(_))
    ) {
        return false;
    }

    if parsed.destination_ip != config.group {
        return false;
    }

    match config.source {
        crate::SourceFilter::Any => true,
        crate::SourceFilter::Source(source) => parsed.source_ip == source,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn raw_packet_from_parts(
    subscription_id: SubscriptionId,
    datagram: &[u8],
    parsed: ParsedIpDatagram,
    metadata: ReceiveMetadata,
) -> RawPacket {
    RawPacket {
        subscription_id,
        datagram: Bytes::copy_from_slice(&datagram[..parsed.datagram_len]),
        source_ip: Some(parsed.source_ip),
        group: Some(parsed.destination_ip),
        ip_protocol: Some(parsed.protocol),
        metadata,
    }
}

#[cfg(target_os = "macos")]
fn next_matching_apple_bpf_packet(
    packet_block: &[u8],
    start_offset: usize,
    datalink: u32,
    interface_index: Option<u32>,
    subscription_id: SubscriptionId,
    config: &RawSubscriptionConfig,
) -> Result<(Option<RawPacket>, usize), McrxError> {
    let mut offset = start_offset;
    let header_len = std::mem::size_of::<libc::bpf_hdr>();

    while offset + header_len <= packet_block.len() {
        let header = unsafe {
            std::ptr::read_unaligned(packet_block[offset..].as_ptr().cast::<libc::bpf_hdr>())
        };
        let record_len = (header.bh_hdrlen as usize).checked_add(header.bh_caplen as usize);
        let Some(record_len) = record_len else {
            return Ok((None, packet_block.len()));
        };
        let next_offset = offset
            .checked_add(bpf_word_align(record_len))
            .unwrap_or(packet_block.len());

        if (header.bh_hdrlen as usize) < header_len {
            return Ok((None, packet_block.len()));
        }

        let Some(data_start) = offset.checked_add(header.bh_hdrlen as usize) else {
            return Ok((None, packet_block.len()));
        };
        let Some(data_end) = data_start.checked_add(header.bh_caplen as usize) else {
            return Ok((None, packet_block.len()));
        };

        if data_end > packet_block.len() {
            return Ok((None, packet_block.len()));
        }

        let frame = &packet_block[data_start..data_end];
        let Some(datagram) = strip_apple_link_layer(frame, datalink)? else {
            offset = next_offset;
            continue;
        };

        let Some(parsed) = parse_ip_datagram(datagram) else {
            offset = next_offset;
            continue;
        };

        if packet_matches_config(parsed, config) {
            let mut metadata = ReceiveMetadata::empty();
            metadata.configured_interface = config.interface;
            metadata.configured_interface_index = config.interface_index;
            metadata.destination_local_ip = Some(parsed.destination_ip);
            metadata.ingress_interface_index = interface_index;

            return Ok((
                Some(raw_packet_from_parts(
                    subscription_id,
                    datagram,
                    parsed,
                    metadata,
                )),
                next_offset,
            ));
        }

        offset = next_offset;
    }

    Ok((None, offset))
}

#[cfg(target_os = "macos")]
fn strip_apple_link_layer(frame: &[u8], datalink: u32) -> Result<Option<&[u8]>, McrxError> {
    match datalink {
        libc::DLT_RAW => Ok(Some(frame)),
        libc::DLT_NULL | libc::DLT_LOOP => Ok(frame.get(4..)),
        libc::DLT_EN10MB => Ok(strip_ethernet_link_layer(frame)),
        other => Err(McrxError::RawPacketReceiveUnsupported(format!(
            "macOS BPF datalink type {other} is not supported yet"
        ))),
    }
}

#[cfg(any(target_os = "macos", test))]
fn strip_ethernet_link_layer(frame: &[u8]) -> Option<&[u8]> {
    if frame.len() < 14 {
        return None;
    }

    let mut ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    let mut payload_offset = 14;

    while matches!(ethertype, 0x8100 | 0x88a8 | 0x9100) {
        if frame.len() < payload_offset + 4 {
            return None;
        }
        ethertype = u16::from_be_bytes([frame[payload_offset + 2], frame[payload_offset + 3]]);
        payload_offset += 4;
    }

    match ethertype {
        0x0800 | 0x86dd => frame.get(payload_offset..),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn bpf_word_align(value: usize) -> usize {
    let alignment = libc::BPF_ALIGNMENT as usize;
    (value + alignment - 1) & !(alignment - 1)
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn parse_ip_datagram(datagram: &[u8]) -> Option<ParsedIpDatagram> {
    let version = datagram.first().map(|byte| byte >> 4)?;
    match version {
        4 => parse_ipv4_datagram(datagram),
        6 => parse_ipv6_datagram(datagram),
        _ => None,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn parse_ipv4_datagram(datagram: &[u8]) -> Option<ParsedIpDatagram> {
    if datagram.len() < 20 {
        return None;
    }

    let ihl = ((datagram[0] & 0x0f) as usize) * 4;
    if ihl < 20 || datagram.len() < ihl {
        return None;
    }

    let datagram_len = u16::from_be_bytes([datagram[2], datagram[3]]) as usize;
    if datagram_len < ihl || datagram.len() < datagram_len {
        return None;
    }

    Some(ParsedIpDatagram {
        source_ip: IpAddr::V4(Ipv4Addr::new(
            datagram[12],
            datagram[13],
            datagram[14],
            datagram[15],
        )),
        destination_ip: IpAddr::V4(Ipv4Addr::new(
            datagram[16],
            datagram[17],
            datagram[18],
            datagram[19],
        )),
        protocol: datagram[9],
        datagram_len,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn parse_ipv6_datagram(datagram: &[u8]) -> Option<ParsedIpDatagram> {
    if datagram.len() < 40 {
        return None;
    }

    let source = <[u8; 16]>::try_from(&datagram[8..24]).ok()?;
    let destination = <[u8; 16]>::try_from(&datagram[24..40]).ok()?;
    let payload_len = u16::from_be_bytes([datagram[4], datagram[5]]) as usize;

    // A zero payload length with a hop-by-hop header can identify a jumbogram,
    // which this bounded receive API does not claim to support.
    if payload_len == 0 && datagram[6] == 0 {
        return None;
    }

    let datagram_len = 40usize.checked_add(payload_len)?;
    if datagram.len() < datagram_len {
        return None;
    }

    Some(ParsedIpDatagram {
        source_ip: IpAddr::V6(Ipv6Addr::from(source)),
        destination_ip: IpAddr::V6(Ipv6Addr::from(destination)),
        protocol: datagram[6],
        datagram_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn run_bpf_filter(instructions: &[ClassicBpfInstruction], packet: &[u8]) -> u32 {
        let mut accumulator = 0u32;
        let mut pc = 0usize;

        loop {
            let instruction = instructions[pc];
            match instruction.code {
                code if code == (BPF_LD | BPF_B | BPF_ABS) => {
                    accumulator = packet[instruction.k as usize] as u32;
                    pc += 1;
                }
                code if code == (BPF_LD | BPF_W | BPF_ABS) => {
                    let offset = instruction.k as usize;
                    accumulator = u32::from_be_bytes(
                        packet[offset..offset + 4]
                            .try_into()
                            .expect("four-byte BPF load"),
                    );
                    pc += 1;
                }
                code if code == (BPF_ALU | BPF_AND | BPF_K) => {
                    accumulator &= instruction.k;
                    pc += 1;
                }
                code if code == (BPF_JMP | BPF_JEQ | BPF_K) => {
                    let jump = if accumulator == instruction.k {
                        instruction.jt
                    } else {
                        instruction.jf
                    };
                    pc += jump as usize + 1;
                }
                code if code == (BPF_RET | BPF_K) => return instruction.k,
                code => panic!("unsupported test BPF instruction {code:#x}"),
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn kernel_filter_enforces_group_and_ssm_source() {
        let mut datagram = [0u8; 20];
        datagram[0] = 0x45;
        datagram[2..4].copy_from_slice(&20u16.to_be_bytes());
        datagram[9] = 17;
        datagram[12..16].copy_from_slice(&[10, 1, 2, 3]);
        datagram[16..20].copy_from_slice(&[232, 1, 2, 3]);
        let config =
            RawSubscriptionConfig::ssm(Ipv4Addr::new(232, 1, 2, 3), Ipv4Addr::new(10, 1, 2, 3));
        let filter = bpf_ip_filter(&config, 0);

        assert_ne!(run_bpf_filter(&filter, &datagram), 0);

        datagram[15] = 4;
        assert_eq!(run_bpf_filter(&filter, &datagram), 0);
        datagram[15] = 3;
        datagram[19] = 4;
        assert_eq!(run_bpf_filter(&filter, &datagram), 0);
    }

    #[cfg(all(target_os = "linux", feature = "raw-shared-capture"))]
    #[test]
    fn shared_capture_filter_rejects_other_ip_families_and_unicast() {
        let mut ipv4 = [0u8; 20];
        ipv4[0] = 0x45;
        ipv4[16..20].copy_from_slice(&[239, 1, 2, 3]);
        let ipv4_filter = bpf_ip_multicast_family_filter(SubscriptionAddressFamily::Ipv4, 0);

        assert_ne!(run_bpf_filter(&ipv4_filter, &ipv4), 0);
        ipv4[0] = 0x60;
        assert_eq!(run_bpf_filter(&ipv4_filter, &ipv4), 0);
        ipv4[0] = 0x45;
        ipv4[16] = 192;
        assert_eq!(run_bpf_filter(&ipv4_filter, &ipv4), 0);

        let mut ipv6 = [0u8; 40];
        ipv6[0] = 0x60;
        ipv6[24] = 0xff;
        let ipv6_filter = bpf_ip_multicast_family_filter(SubscriptionAddressFamily::Ipv6, 0);

        assert_ne!(run_bpf_filter(&ipv6_filter, &ipv6), 0);
        ipv6[0] = 0x45;
        assert_eq!(run_bpf_filter(&ipv6_filter, &ipv6), 0);
        ipv6[0] = 0x60;
        ipv6[24] = 0x20;
        assert_eq!(run_bpf_filter(&ipv6_filter, &ipv6), 0);
    }

    #[cfg(target_os = "macos")]
    fn append_bpf_record(block: &mut Vec<u8>, datagram: &[u8]) {
        let mut header = unsafe { std::mem::zeroed::<libc::bpf_hdr>() };
        header.bh_hdrlen = std::mem::size_of::<libc::bpf_hdr>() as _;
        header.bh_caplen = datagram.len() as _;
        header.bh_datalen = datagram.len() as _;

        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                (&header as *const libc::bpf_hdr).cast::<u8>(),
                std::mem::size_of::<libc::bpf_hdr>(),
            )
        };
        let record_len = header_bytes.len() + datagram.len();
        block.extend_from_slice(header_bytes);
        block.extend_from_slice(datagram);
        block.resize(block.len() + bpf_word_align(record_len) - record_len, 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bpf_batch_preserves_every_matching_record() {
        fn datagram(source_last_octet: u8) -> [u8; 20] {
            let mut datagram = [0u8; 20];
            datagram[0] = 0x45;
            datagram[2..4].copy_from_slice(&20u16.to_be_bytes());
            datagram[9] = 17;
            datagram[12..16].copy_from_slice(&[10, 1, 2, source_last_octet]);
            datagram[16..20].copy_from_slice(&[239, 1, 2, 3]);
            datagram
        }

        let mut block = Vec::new();
        append_bpf_record(&mut block, &datagram(3));
        append_bpf_record(&mut block, &datagram(4));
        let config = RawSubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3));

        let (first, offset) = next_matching_apple_bpf_packet(
            &block,
            0,
            libc::DLT_RAW,
            Some(7),
            SubscriptionId(1),
            &config,
        )
        .unwrap();
        let (second, end) = next_matching_apple_bpf_packet(
            &block,
            offset,
            libc::DLT_RAW,
            Some(7),
            SubscriptionId(1),
            &config,
        )
        .unwrap();

        assert_eq!(
            first.unwrap().source_ip,
            Some(Ipv4Addr::new(10, 1, 2, 3).into())
        );
        assert_eq!(
            second.unwrap().source_ip,
            Some(Ipv4Addr::new(10, 1, 2, 4).into())
        );
        assert_eq!(end, block.len());
    }

    #[test]
    fn parses_ipv4_datagram_fields() {
        let datagram = [
            0x45, 0x00, 0x00, 0x1c, 0x12, 0x34, 0x00, 0x00, 0x01, 0x11, 0x00, 0x00, 10, 1, 2, 3,
            239, 1, 2, 3, 1, 2, 3, 4, 5, 6, 7, 8,
        ];

        let parsed = parse_ip_datagram(&datagram).unwrap();
        assert_eq!(parsed.source_ip, IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)));
        assert_eq!(
            parsed.destination_ip,
            IpAddr::V4(Ipv4Addr::new(239, 1, 2, 3))
        );
        assert_eq!(parsed.protocol, 17);
    }

    #[test]
    fn parses_ipv6_datagram_fields() {
        let mut datagram = [0u8; 40];
        datagram[0] = 0x60;
        datagram[6] = 17;
        datagram[8..24].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());
        datagram[24..40].copy_from_slice(&"ff3e::8000:1234".parse::<Ipv6Addr>().unwrap().octets());

        let parsed = parse_ip_datagram(&datagram).unwrap();
        assert_eq!(parsed.source_ip, IpAddr::V6(Ipv6Addr::LOCALHOST));
        assert_eq!(
            parsed.destination_ip,
            IpAddr::V6("ff3e::8000:1234".parse::<Ipv6Addr>().unwrap())
        );
        assert_eq!(parsed.protocol, 17);
    }

    #[test]
    fn malformed_datagram_is_rejected() {
        assert!(parse_ip_datagram(&[0x45, 0x00, 0x00]).is_none());
        assert!(parse_ip_datagram(&[0x70; 8]).is_none());

        let mut truncated_ipv4 = [0u8; 20];
        truncated_ipv4[0] = 0x45;
        truncated_ipv4[2..4].copy_from_slice(&28u16.to_be_bytes());
        assert!(parse_ip_datagram(&truncated_ipv4).is_none());

        let mut truncated_ipv6 = [0u8; 40];
        truncated_ipv6[0] = 0x60;
        truncated_ipv6[4..6].copy_from_slice(&8u16.to_be_bytes());
        assert!(parse_ip_datagram(&truncated_ipv6).is_none());
    }

    #[test]
    fn raw_packet_trims_link_layer_padding_to_ip_length() {
        let mut datagram = vec![0u8; 32];
        datagram[0] = 0x45;
        datagram[2..4].copy_from_slice(&20u16.to_be_bytes());
        datagram[9] = 17;
        datagram[12..16].copy_from_slice(&[10, 1, 2, 3]);
        datagram[16..20].copy_from_slice(&[239, 1, 2, 3]);
        let parsed = parse_ip_datagram(&datagram).unwrap();

        let packet = raw_packet_from_parts(
            SubscriptionId(1),
            &datagram,
            parsed,
            ReceiveMetadata::empty(),
        );

        assert_eq!(packet.datagram.len(), 20);
    }

    #[test]
    fn packet_filter_rejects_ipv6_for_ipv4_subscription() {
        let parsed = ParsedIpDatagram {
            source_ip: IpAddr::V6(Ipv6Addr::LOCALHOST),
            destination_ip: IpAddr::V6("ff3e::8000:1234".parse().unwrap()),
            protocol: 17,
            datagram_len: 40,
        };
        let config = RawSubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3));

        assert!(!packet_matches_config(parsed, &config));
    }

    #[test]
    fn packet_filter_rejects_ipv4_for_ipv6_subscription() {
        let parsed = ParsedIpDatagram {
            source_ip: IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)),
            destination_ip: IpAddr::V4(Ipv4Addr::new(239, 1, 2, 3)),
            protocol: 17,
            datagram_len: 20,
        };
        let config = RawSubscriptionConfig::asm_v6("ff12::8000:1234".parse().unwrap());

        assert!(!packet_matches_config(parsed, &config));
    }

    #[test]
    fn strips_ethernet_ipv4_frame() {
        let frame = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0x08, 0x00, 0x45, 0, 0, 20, 0, 0, 0, 0, 1, 17, 0,
            0, 10, 1, 2, 3, 239, 1, 2, 3,
        ];

        let datagram = strip_ethernet_link_layer(&frame).unwrap();
        let parsed = parse_ip_datagram(datagram).unwrap();

        assert_eq!(parsed.source_ip, IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)));
        assert_eq!(
            parsed.destination_ip,
            IpAddr::V4(Ipv4Addr::new(239, 1, 2, 3))
        );
    }

    #[test]
    fn strips_vlan_ethernet_ipv6_frame() {
        let mut frame = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0x81, 0x00, 0, 1, 0x86, 0xdd,
        ];
        let mut datagram = [0u8; 40];
        datagram[0] = 0x60;
        datagram[6] = 17;
        datagram[8..24].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());
        datagram[24..40].copy_from_slice(&"ff3e::8000:1234".parse::<Ipv6Addr>().unwrap().octets());
        frame.extend_from_slice(&datagram);

        let stripped = strip_ethernet_link_layer(&frame).unwrap();
        let parsed = parse_ip_datagram(stripped).unwrap();

        assert_eq!(parsed.source_ip, IpAddr::V6(Ipv6Addr::LOCALHOST));
        assert_eq!(
            parsed.destination_ip,
            IpAddr::V6("ff3e::8000:1234".parse::<Ipv6Addr>().unwrap())
        );
    }

    #[test]
    fn ignores_non_ip_ethernet_frame() {
        let frame = [0u8; 14];
        assert!(strip_ethernet_link_layer(&frame).is_none());
    }
}
