use crate::config::{SourceFilter, SubscriptionConfig};
use crate::error::McrxError;
use crate::packet::{Packet, PacketWithMetadata, ReceiveMetadata};
use crate::subscription::SubscriptionId;
use bytes::Bytes;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(windows)]
use std::os::windows::io::AsRawSocket;
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::{
    CMSGHDR, IN_ADDR, IN_PKTINFO, IP_PKTINFO, IPPROTO_IP, LPFN_WSARECVMSG,
    LPWSAOVERLAPPED_COMPLETION_ROUTINE, SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR, SOCKET,
    SOCKET_ERROR, WSABUF, WSAGetLastError, WSAID_WSARECVMSG, WSAIoctl, WSAMSG, setsockopt,
};
#[cfg(windows)]
use windows_sys::Win32::System::IO::OVERLAPPED;

fn resolve_interface(config: &SubscriptionConfig) -> Ipv4Addr {
    config.interface.unwrap_or(Ipv4Addr::UNSPECIFIED)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "wasi"
    ))
))]
fn set_port_reuse_if_supported(socket: &Socket) -> std::io::Result<()> {
    socket.set_reuse_port(true)
}

#[cfg(not(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "wasi"
    ))
)))]
fn set_port_reuse_if_supported(_socket: &Socket) -> std::io::Result<()> {
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailedReceiveMode {
    Basic,
    Ancillary,
}

#[cfg(windows)]
type WsaRecvMsgFn = unsafe extern "system" fn(
    SOCKET,
    *mut WSAMSG,
    *mut u32,
    *mut OVERLAPPED,
    LPWSAOVERLAPPED_COMPLETION_ROUTINE,
) -> i32;

pub(crate) struct ReceiveSocket {
    socket: Socket,
    local_addr: Option<SocketAddr>,
    detailed_receive_mode: DetailedReceiveMode,
    #[cfg(windows)]
    wsarecvmsg: Option<WsaRecvMsgFn>,
}

impl std::fmt::Debug for ReceiveSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceiveSocket")
            .field("local_addr", &self.local_addr)
            .field("detailed_receive_mode", &self.detailed_receive_mode)
            .finish_non_exhaustive()
    }
}

impl ReceiveSocket {
    pub(crate) fn adopt(socket: Socket) -> Self {
        let local_addr = try_socket_local_addr(&socket).ok();

        #[cfg(windows)]
        let (detailed_receive_mode, wsarecvmsg) = configure_detailed_receive(&socket);
        #[cfg(not(windows))]
        let detailed_receive_mode = configure_detailed_receive(&socket);

        Self {
            socket,
            local_addr,
            detailed_receive_mode,
            #[cfg(windows)]
            wsarecvmsg,
        }
    }

    pub(crate) fn socket(&self) -> &Socket {
        &self.socket
    }

    pub(crate) fn socket_mut(&mut self) -> &mut Socket {
        &mut self.socket
    }

    pub(crate) fn local_addr(&self) -> Result<SocketAddr, McrxError> {
        self.local_addr
            .or_else(|| try_socket_local_addr(&self.socket).ok())
            .ok_or_else(|| {
                McrxError::SocketLocalAddrFailed(std::io::Error::other(
                    "failed to determine socket local address",
                ))
            })
    }

    fn base_metadata(&self, config: &SubscriptionConfig) -> Result<ReceiveMetadata, McrxError> {
        Ok(ReceiveMetadata {
            socket_local_addr: Some(self.local_addr()?),
            configured_interface: config.interface.map(IpAddr::V4),
            destination_local_ip: None,
            ingress_interface_index: None,
        })
    }

    fn uses_ancillary_metadata(&self) -> bool {
        matches!(self.detailed_receive_mode, DetailedReceiveMode::Ancillary)
    }

    #[cfg(windows)]
    fn wsarecvmsg(&self) -> Option<WsaRecvMsgFn> {
        self.wsarecvmsg
    }

    pub(crate) fn into_socket(self) -> Socket {
        self.socket
    }
}

#[cfg(windows)]
fn raw_socket(socket: &Socket) -> SOCKET {
    socket.as_raw_socket() as SOCKET
}

#[cfg(windows)]
fn last_wsa_error() -> std::io::Error {
    std::io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
}

fn try_socket_local_addr(socket: &Socket) -> Result<SocketAddr, McrxError> {
    socket
        .local_addr()
        .map_err(McrxError::SocketLocalAddrFailed)?
        .as_socket()
        .ok_or(McrxError::NonIpSocketAddress)
}

#[cfg(windows)]
fn set_socket_option_flag(socket: &Socket, level: i32, name: i32) -> Result<(), McrxError> {
    let enabled: u32 = 1;
    let result = unsafe {
        setsockopt(
            raw_socket(socket),
            level,
            name,
            (&enabled as *const u32).cast(),
            std::mem::size_of_val(&enabled) as i32,
        )
    };

    if result == SOCKET_ERROR {
        Err(McrxError::SocketOptionFailed(last_wsa_error()))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn get_wsarecvmsg(socket: &Socket) -> Result<WsaRecvMsgFn, McrxError> {
    let mut bytes_returned = 0u32;
    let guid = WSAID_WSARECVMSG;
    let mut func: LPFN_WSARECVMSG = None;

    let result = unsafe {
        WSAIoctl(
            raw_socket(socket),
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            (&guid as *const windows_sys::core::GUID).cast_mut().cast(),
            std::mem::size_of_val(&guid) as u32,
            (&mut func as *mut LPFN_WSARECVMSG).cast(),
            std::mem::size_of_val(&func) as u32,
            &mut bytes_returned,
            std::ptr::null_mut(),
            None,
        )
    };

    if result == SOCKET_ERROR {
        return Err(McrxError::SocketIoctlFailed(last_wsa_error()));
    }

    match func {
        Some(func) => Ok(func),
        None => Err(McrxError::SocketIoctlFailed(std::io::Error::other(
            "WSARecvMsg lookup returned null",
        ))),
    }
}

#[cfg(unix)]
fn set_socket_option_flag(
    socket: &Socket,
    level: libc::c_int,
    name: libc::c_int,
) -> Result<(), McrxError> {
    let enabled: libc::c_int = 1;
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            level,
            name,
            (&enabled as *const libc::c_int).cast(),
            std::mem::size_of_val(&enabled) as libc::socklen_t,
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

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
fn enable_receive_metadata(socket: &Socket) -> Result<(), McrxError> {
    set_socket_option_flag(socket, libc::IPPROTO_IP, libc::IP_PKTINFO)
}

#[cfg(all(
    unix,
    any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
fn enable_receive_metadata(socket: &Socket) -> Result<(), McrxError> {
    set_socket_option_flag(socket, libc::IPPROTO_IP, libc::IP_RECVDSTADDR)?;
    set_socket_option_flag(socket, libc::IPPROTO_IP, libc::IP_RECVIF)
}

#[cfg(windows)]
fn enable_receive_metadata(socket: &Socket) -> Result<(), McrxError> {
    set_socket_option_flag(socket, IPPROTO_IP as i32, IP_PKTINFO)
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
fn configure_detailed_receive(socket: &Socket) -> DetailedReceiveMode {
    if enable_receive_metadata(socket).is_ok() {
        DetailedReceiveMode::Ancillary
    } else {
        DetailedReceiveMode::Basic
    }
}

#[cfg(all(
    unix,
    any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
fn configure_detailed_receive(socket: &Socket) -> DetailedReceiveMode {
    if enable_receive_metadata(socket).is_ok() {
        DetailedReceiveMode::Ancillary
    } else {
        DetailedReceiveMode::Basic
    }
}

#[cfg(windows)]
fn configure_detailed_receive(socket: &Socket) -> (DetailedReceiveMode, Option<WsaRecvMsgFn>) {
    if enable_receive_metadata(socket).is_err() {
        return (DetailedReceiveMode::Basic, None);
    }

    match get_wsarecvmsg(socket) {
        Ok(recvmsg) => (DetailedReceiveMode::Ancillary, Some(recvmsg)),
        Err(_) => (DetailedReceiveMode::Basic, None),
    }
}

#[cfg(not(any(
    windows,
    all(unix, any(target_os = "linux", target_os = "android")),
    all(
        unix,
        any(
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "netbsd",
            target_os = "openbsd"
        )
    )
)))]
fn configure_detailed_receive(_socket: &Socket) -> DetailedReceiveMode {
    DetailedReceiveMode::Basic
}

#[cfg(unix)]
fn ipv4_from_in_addr(addr: libc::in_addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from_be(addr.s_addr))
}

#[cfg(windows)]
fn ipv4_from_in_addr(addr: IN_ADDR) -> Ipv4Addr {
    let raw = unsafe { addr.S_un.S_addr };
    Ipv4Addr::from(u32::from_be(raw))
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
fn ancillary_buffer_size() -> usize {
    unsafe { libc::CMSG_SPACE(std::mem::size_of::<libc::in_pktinfo>() as libc::c_uint) as usize }
}

#[cfg(all(
    unix,
    any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
fn ancillary_buffer_size() -> usize {
    unsafe {
        let dst = libc::CMSG_SPACE(std::mem::size_of::<libc::in_addr>() as libc::c_uint);
        let iface = libc::CMSG_SPACE(std::mem::size_of::<libc::sockaddr_dl>() as libc::c_uint);
        (dst + iface) as usize
    }
}

#[cfg(windows)]
fn wsa_cmsg_align(length: usize) -> usize {
    let align = std::mem::align_of::<CMSGHDR>();
    (length + align - 1) & !(align - 1)
}

#[cfg(windows)]
fn wsa_cmsg_space(length: usize) -> usize {
    wsa_cmsg_align(std::mem::size_of::<CMSGHDR>()) + wsa_cmsg_align(length)
}

#[cfg(windows)]
fn wsa_cmsg_len(length: usize) -> usize {
    wsa_cmsg_align(std::mem::size_of::<CMSGHDR>()) + length
}

#[cfg(windows)]
fn ancillary_buffer_size() -> usize {
    wsa_cmsg_space(std::mem::size_of::<IN_PKTINFO>())
}

#[cfg(not(any(
    windows,
    all(unix, any(target_os = "linux", target_os = "android")),
    all(
        unix,
        any(
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "netbsd",
            target_os = "openbsd"
        )
    )
)))]
fn ancillary_buffer_size() -> usize {
    0
}

#[cfg(unix)]
unsafe fn control_message_contains<T>(cmsg: *const libc::cmsghdr) -> bool {
    unsafe {
        (*cmsg).cmsg_len as usize
            >= libc::CMSG_LEN(std::mem::size_of::<T>() as libc::c_uint) as usize
    }
}

#[cfg(windows)]
unsafe fn control_message_contains<T>(cmsg: *const CMSGHDR) -> bool {
    unsafe { (*cmsg).cmsg_len >= wsa_cmsg_len(std::mem::size_of::<T>()) }
}

#[cfg(windows)]
fn wsa_cmsg_firsthdr(msg: &WSAMSG) -> *mut CMSGHDR {
    if msg.Control.buf.is_null() || (msg.Control.len as usize) < wsa_cmsg_space(0) {
        std::ptr::null_mut()
    } else {
        msg.Control.buf.cast()
    }
}

#[cfg(windows)]
unsafe fn wsa_cmsg_nxthdr(msg: &WSAMSG, cmsg: *const CMSGHDR) -> *mut CMSGHDR {
    if cmsg.is_null() {
        return wsa_cmsg_firsthdr(msg);
    }

    let next = (cmsg as usize + wsa_cmsg_align(unsafe { (*cmsg).cmsg_len })) as *mut CMSGHDR;
    let max = msg.Control.buf as usize + msg.Control.len as usize;

    if next as usize + wsa_cmsg_align(std::mem::size_of::<CMSGHDR>()) > max {
        std::ptr::null_mut()
    } else {
        next
    }
}

#[cfg(windows)]
unsafe fn wsa_cmsg_data(cmsg: *const CMSGHDR) -> *mut u8 {
    unsafe { (cmsg as *mut u8).add(wsa_cmsg_align(std::mem::size_of::<CMSGHDR>())) }
}

#[cfg(all(
    unix,
    any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
#[repr(C)]
#[derive(Clone, Copy)]
struct SockAddrDlPrefix {
    sdl_len: libc::c_uchar,
    sdl_family: libc::c_uchar,
    sdl_index: libc::c_ushort,
}

#[cfg(unix)]
fn apply_receive_ancillary_data(msg: &libc::msghdr, metadata: &mut ReceiveMetadata) {
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(msg);

        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::IPPROTO_IP {
                #[cfg(any(target_os = "linux", target_os = "android"))]
                if (*cmsg).cmsg_type == libc::IP_PKTINFO
                    && control_message_contains::<libc::in_pktinfo>(cmsg)
                {
                    let pktinfo =
                        std::ptr::read_unaligned(libc::CMSG_DATA(cmsg) as *const libc::in_pktinfo);
                    metadata.destination_local_ip =
                        Some(IpAddr::V4(ipv4_from_in_addr(pktinfo.ipi_addr)));
                    if pktinfo.ipi_ifindex != 0 {
                        metadata.ingress_interface_index = Some(pktinfo.ipi_ifindex as u32);
                    }
                }

                #[cfg(any(
                    target_vendor = "apple",
                    target_os = "freebsd",
                    target_os = "dragonfly",
                    target_os = "netbsd",
                    target_os = "openbsd"
                ))]
                match (*cmsg).cmsg_type {
                    libc::IP_RECVDSTADDR if control_message_contains::<libc::in_addr>(cmsg) => {
                        let addr =
                            std::ptr::read_unaligned(libc::CMSG_DATA(cmsg) as *const libc::in_addr);
                        metadata.destination_local_ip = Some(IpAddr::V4(ipv4_from_in_addr(addr)));
                    }
                    libc::IP_RECVIF if control_message_contains::<SockAddrDlPrefix>(cmsg) => {
                        let addr = std::ptr::read_unaligned(
                            libc::CMSG_DATA(cmsg) as *const SockAddrDlPrefix
                        );
                        if addr.sdl_index != 0 {
                            metadata.ingress_interface_index = Some(addr.sdl_index as u32);
                        }
                    }
                    _ => {}
                }
            }

            cmsg = libc::CMSG_NXTHDR(msg, cmsg);
        }
    }
}

#[cfg(windows)]
fn apply_receive_ancillary_data(msg: &WSAMSG, metadata: &mut ReceiveMetadata) {
    unsafe {
        let mut cmsg = wsa_cmsg_firsthdr(msg);

        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == IPPROTO_IP
                && (*cmsg).cmsg_type == IP_PKTINFO
                && control_message_contains::<IN_PKTINFO>(cmsg)
            {
                let pktinfo = std::ptr::read_unaligned(wsa_cmsg_data(cmsg) as *const IN_PKTINFO);
                metadata.destination_local_ip =
                    Some(IpAddr::V4(ipv4_from_in_addr(pktinfo.ipi_addr)));
                if pktinfo.ipi_ifindex != 0 {
                    metadata.ingress_interface_index = Some(pktinfo.ipi_ifindex);
                }
            }

            cmsg = wsa_cmsg_nxthdr(msg, cmsg);
        }
    }
}

#[cfg(unix)]
fn recv_packet_with_metadata_unix(
    socket: &ReceiveSocket,
    subscription_id: SubscriptionId,
    config: &SubscriptionConfig,
) -> Result<Option<PacketWithMetadata>, McrxError> {
    let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); 65535];
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    let mut control = vec![std::mem::MaybeUninit::<u8>::uninit(); ancillary_buffer_size()];
    let mut control_len = 0usize;

    let (len, addr) = match unsafe {
        SockAddr::try_init(|addr_storage, addr_len| {
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_name = addr_storage.cast();
            msg.msg_namelen = *addr_len;
            msg.msg_iov = std::ptr::addr_of_mut!(iov);
            msg.msg_iovlen = 1;

            if !control.is_empty() {
                msg.msg_control = control.as_mut_ptr().cast();
                msg.msg_controllen = control.len() as _;
            }

            let received = libc::recvmsg(socket.socket().as_raw_fd(), &mut msg, 0);
            if received == -1 {
                return Err(std::io::Error::last_os_error());
            }

            *addr_len = msg.msg_namelen;
            control_len = msg.msg_controllen as usize;

            Ok(received as usize)
        })
    } {
        Ok(result) => result,
        Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(None),
        Err(err) => return Err(McrxError::ReceiveFailed(err)),
    };

    let source = addr.as_socket().ok_or(McrxError::NonIpSocketAddress)?;

    // SAFETY: `recvmsg` initialized exactly the first `len` bytes of `buf`.
    // We only create a slice over that initialized prefix, and copy it immediately.
    let payload_bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, len) };

    let mut metadata = socket.base_metadata(config)?;

    if control_len != 0 {
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = control_len as _;
        apply_receive_ancillary_data(&msg, &mut metadata);
    }

    Ok(Some(PacketWithMetadata {
        packet: Packet {
            subscription_id,
            source,
            group: IpAddr::V4(config.group),
            dst_port: config.dst_port,
            payload: Bytes::copy_from_slice(payload_bytes),
        },
        metadata,
    }))
}

#[cfg(windows)]
fn recv_packet_with_metadata_windows(
    socket: &ReceiveSocket,
    subscription_id: SubscriptionId,
    config: &SubscriptionConfig,
) -> Result<Option<PacketWithMetadata>, McrxError> {
    let recvmsg = match socket.wsarecvmsg() {
        Some(recvmsg) => recvmsg,
        None => return recv_packet_impl(socket, subscription_id, config, true),
    };
    let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); 65535];
    let mut data_buf = WSABUF {
        len: buf.len() as u32,
        buf: buf.as_mut_ptr().cast(),
    };
    let mut control = vec![std::mem::MaybeUninit::<u8>::uninit(); ancillary_buffer_size()];
    let mut bytes_received = 0u32;

    let (msg, addr) = match unsafe {
        SockAddr::try_init(|addr_storage, addr_len| {
            let mut msg = WSAMSG::default();
            msg.name = addr_storage.cast::<SOCKADDR>();
            msg.namelen = *addr_len as i32;
            msg.lpBuffers = std::ptr::addr_of_mut!(data_buf);
            msg.dwBufferCount = 1;

            if !control.is_empty() {
                msg.Control = WSABUF {
                    len: control.len() as u32,
                    buf: control.as_mut_ptr().cast(),
                };
            }

            let result = recvmsg(
                raw_socket(socket.socket()),
                &mut msg,
                &mut bytes_received,
                std::ptr::null_mut(),
                None,
            );

            if result == SOCKET_ERROR {
                return Err(last_wsa_error());
            }

            *addr_len = msg.namelen as _;

            Ok(msg)
        })
    } {
        Ok(result) => result,
        Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(None),
        Err(err) => return Err(McrxError::ReceiveFailed(err)),
    };

    let source = addr.as_socket().ok_or(McrxError::NonIpSocketAddress)?;

    // SAFETY: `WSARecvMsg` initialized exactly the first `bytes_received` bytes of `buf`.
    // We only create a slice over that initialized prefix, and copy it immediately.
    let payload_bytes =
        unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, bytes_received as usize) };

    let mut metadata = socket.base_metadata(config)?;

    if !msg.Control.buf.is_null() && msg.Control.len != 0 {
        apply_receive_ancillary_data(&msg, &mut metadata);
    }

    Ok(Some(PacketWithMetadata {
        packet: Packet {
            subscription_id,
            source,
            group: IpAddr::V4(config.group),
            dst_port: config.dst_port,
            payload: Bytes::copy_from_slice(payload_bytes),
        },
        metadata,
    }))
}

#[cfg(unix)]
fn recv_packet_with_ancillary_metadata(
    socket: &ReceiveSocket,
    subscription_id: SubscriptionId,
    config: &SubscriptionConfig,
) -> Result<Option<PacketWithMetadata>, McrxError> {
    match recv_packet_with_metadata_unix(socket, subscription_id, config) {
        Err(McrxError::ReceiveFailed(err)) if err.kind() == ErrorKind::WouldBlock => Ok(None),
        other => other,
    }
}

#[cfg(windows)]
fn recv_packet_with_ancillary_metadata(
    socket: &ReceiveSocket,
    subscription_id: SubscriptionId,
    config: &SubscriptionConfig,
) -> Result<Option<PacketWithMetadata>, McrxError> {
    match recv_packet_with_metadata_windows(socket, subscription_id, config) {
        Err(McrxError::ReceiveFailed(err)) if err.kind() == ErrorKind::WouldBlock => Ok(None),
        other => other,
    }
}

#[cfg(not(any(unix, windows)))]
fn recv_packet_with_ancillary_metadata(
    socket: &ReceiveSocket,
    subscription_id: SubscriptionId,
    config: &SubscriptionConfig,
) -> Result<Option<PacketWithMetadata>, McrxError> {
    recv_packet_impl(socket, subscription_id, config, true)
}

/// Opens and binds a UDP socket for the given subscription configuration.
///
/// The socket is bound to `0.0.0.0:dst_port` so it can receive multicast traffic
/// destined for the configured UDP port. The socket is configured as non-blocking.
pub(crate) fn open_bound_socket(config: &SubscriptionConfig) -> Result<ReceiveSocket, McrxError> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(McrxError::SocketCreateFailed)?;

    socket
        .set_reuse_address(true)
        .map_err(McrxError::SocketOptionFailed)?;

    set_port_reuse_if_supported(&socket).map_err(McrxError::SocketOptionFailed)?;

    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, config.dst_port);

    socket
        .bind(&SockAddr::from(bind_addr))
        .map_err(McrxError::SocketBindFailed)?;

    prepare_existing_socket(socket, config)
}

/// Validates and prepares a caller-provided socket for use with the subscription.
///
/// The socket must already be bound to the destination port from `config`.
/// This helper preserves caller-controlled bind/socket setup while still enforcing
/// the non-blocking contract used by the receive APIs.
pub(crate) fn prepare_existing_socket(
    socket: Socket,
    config: &SubscriptionConfig,
) -> Result<ReceiveSocket, McrxError> {
    let local_addr = socket_local_addr(&socket)?;

    match local_addr {
        SocketAddr::V4(addr) => {
            if addr.port() != config.dst_port {
                return Err(McrxError::ExistingSocketPortMismatch {
                    expected: config.dst_port,
                    actual: addr.port(),
                });
            }
        }
        SocketAddr::V6(_) => {
            return Err(McrxError::ExistingSocketMustBeIpv4);
        }
    }

    socket
        .set_nonblocking(true)
        .map_err(McrxError::SocketOptionFailed)?;

    let mut receive_socket = ReceiveSocket::adopt(socket);
    receive_socket.local_addr = Some(local_addr);
    Ok(receive_socket)
}

/// Returns the local IP socket address for the given socket.
pub(crate) fn socket_local_addr(socket: &Socket) -> Result<SocketAddr, McrxError> {
    try_socket_local_addr(socket)
}

/// Joins the configured multicast group for this subscription on an already bound socket.
///
/// Uses ASM (`(*, G)`) or SSM (`(S, G)`) depending on the configured source filter.
pub(crate) fn join_multicast_group(
    socket: &Socket,
    config: &SubscriptionConfig,
) -> Result<(), McrxError> {
    let interface = resolve_interface(config);

    match config.source {
        SourceFilter::Any => socket
            .join_multicast_v4(&config.group, &interface)
            .map_err(McrxError::MulticastJoinFailed),

        SourceFilter::Source(source) => socket
            .join_ssm_v4(&source, &config.group, &interface)
            .map_err(McrxError::MulticastJoinFailed),
    }
}

/// Leaves the configured multicast group for this subscription on the given socket.
///
/// Uses ASM (`(*, G)`) or SSM (`(S, G)`) depending on the configured source filter.
pub(crate) fn leave_multicast_group(
    socket: &Socket,
    config: &SubscriptionConfig,
) -> Result<(), McrxError> {
    let interface = resolve_interface(config);

    match config.source {
        SourceFilter::Any => socket
            .leave_multicast_v4(&config.group, &interface)
            .map_err(McrxError::MulticastLeaveFailed),

        SourceFilter::Source(source) => socket
            .leave_ssm_v4(&source, &config.group, &interface)
            .map_err(McrxError::MulticastLeaveFailed),
    }
}

fn recv_packet_impl(
    socket: &ReceiveSocket,
    subscription_id: SubscriptionId,
    config: &SubscriptionConfig,
    include_metadata: bool,
) -> Result<Option<PacketWithMetadata>, McrxError> {
    let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); 65535];

    match socket.socket().recv_from(&mut buf) {
        Ok((len, addr)) => {
            let source = addr.as_socket().ok_or(McrxError::NonIpSocketAddress)?;

            // SAFETY: `recv_from` initialized exactly the first `len` bytes of `buf`.
            // We only create a slice over that initialized prefix, and copy it immediately.
            let payload_bytes =
                unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, len) };

            let metadata = if include_metadata {
                socket.base_metadata(config)?
            } else {
                ReceiveMetadata::empty()
            };

            Ok(Some(PacketWithMetadata {
                packet: Packet {
                    subscription_id,
                    source,
                    group: IpAddr::V4(config.group),
                    dst_port: config.dst_port,
                    payload: Bytes::copy_from_slice(payload_bytes),
                },
                metadata,
            }))
        }
        Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(None),
        Err(err) => Err(McrxError::ReceiveFailed(err)),
    }
}

/// Attempts to receive a packet from the given socket without blocking.
pub(crate) fn recv_packet(
    socket: &ReceiveSocket,
    subscription_id: SubscriptionId,
    config: &SubscriptionConfig,
) -> Result<Option<Packet>, McrxError> {
    Ok(recv_packet_impl(socket, subscription_id, config, false)?
        .map(PacketWithMetadata::into_packet))
}

/// Attempts to receive a packet and richer receive metadata without blocking.
pub(crate) fn recv_packet_with_metadata(
    socket: &ReceiveSocket,
    subscription_id: SubscriptionId,
    config: &SubscriptionConfig,
) -> Result<Option<PacketWithMetadata>, McrxError> {
    if socket.uses_ancillary_metadata() {
        return recv_packet_with_ancillary_metadata(socket, subscription_id, config);
    }

    recv_packet_impl(socket, subscription_id, config, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SubscriptionConfig;
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicU16, Ordering};

    static NEXT_TEST_PORT: AtomicU16 = AtomicU16::new(55100);

    fn next_test_port() -> u16 {
        NEXT_TEST_PORT.fetch_add(1, Ordering::Relaxed)
    }

    fn open_socket_on_available_test_port(group: Ipv4Addr) -> (SubscriptionConfig, ReceiveSocket) {
        for _ in 0..128 {
            let config = SubscriptionConfig::asm(group, next_test_port());

            match open_bound_socket(&config) {
                Ok(socket) => return (config, socket),
                Err(McrxError::SocketBindFailed(_)) => continue,
                Err(err) => panic!("failed to open first receive socket: {err:?}"),
            }
        }

        panic!("failed to find an available UDP port for the receive socket test");
    }

    #[test]
    fn open_and_join_socket_succeeds_for_valid_asm_config() {
        let config = SubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3), 55000);

        let socket = open_bound_socket(&config);
        assert!(socket.is_ok());

        let socket = socket.unwrap();
        let result = join_multicast_group(socket.socket(), &config);
        assert!(result.is_ok());
    }

    #[test]
    fn open_and_join_socket_succeeds_for_valid_ssm_config() {
        let config = SubscriptionConfig::ssm(
            Ipv4Addr::new(232, 1, 2, 3),
            Ipv4Addr::new(192, 168, 188, 50),
            55009,
        );

        let socket = open_bound_socket(&config);
        assert!(socket.is_ok());

        let socket = socket.unwrap();
        let result = join_multicast_group(socket.socket(), &config);
        assert!(result.is_ok());
    }

    #[test]
    fn prepare_existing_socket_rejects_wrong_port() {
        let config = SubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3), 55010);

        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
        socket.set_reuse_address(true).unwrap();
        socket
            .bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)))
            .unwrap();

        let result = prepare_existing_socket(socket, &config);

        assert!(matches!(
            result,
            Err(McrxError::ExistingSocketPortMismatch {
                expected: 55010,
                ..
            })
        ));
    }

    #[test]
    #[cfg(all(
        unix,
        not(any(
            target_os = "solaris",
            target_os = "illumos",
            target_os = "cygwin",
            target_os = "wasi"
        ))
    ))]
    fn open_bound_socket_allows_two_receivers_on_same_port() {
        let (config, first) = open_socket_on_available_test_port(Ipv4Addr::new(239, 1, 2, 3));

        assert_eq!(first.local_addr().unwrap().port(), config.dst_port);

        let second = open_bound_socket(&config);
        assert!(second.is_ok(), "second receive socket bind should succeed");
    }
}
