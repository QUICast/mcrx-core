use mcrx_core::{Context, McrxError, PacketWithMetadata, ReceiveMetadata, SubscriptionConfig};
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MCRX_STATUS_OK: c_int = 0;
const MCRX_STATUS_INVALID_ARGUMENT: c_int = 1;
const MCRX_STATUS_ERROR: c_int = 2;
const MCRX_STATUS_ALREADY_RUNNING: c_int = 3;
const MCRX_STATUS_PANIC: c_int = 4;

const DEFAULT_IDLE_SLEEP_MS: u64 = 5;

type McrxPacketCallback =
    unsafe extern "C" fn(packet: *const McrxPacketView, user_data: *mut c_void);

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

#[repr(C)]
pub struct McrxPacketView {
    payload: *const u8,
    payload_len: usize,

    subscription_id: u64,
    source_ip: *const c_char,
    source_port: u16,
    group_ip: *const c_char,
    dst_port: u16,

    socket_local_ip: *const c_char,
    socket_local_port: u16,
    configured_interface_ip: *const c_char,
    configured_interface_index: u32,
    has_configured_interface_index: u8,
    destination_local_ip: *const c_char,
    ingress_interface_index: u32,
    has_ingress_interface_index: u8,
}

#[derive(Debug)]
struct ContextState {
    context: Mutex<Context>,
    last_error: Mutex<Option<CString>>,
    active_worker_generation: AtomicU64,
    next_worker_generation: AtomicU64,
    worker: Mutex<Option<JoinHandle<()>>>,
    idle_wait: Mutex<()>,
    wake_worker: Condvar,
}

#[repr(C)]
pub struct McrxContext {
    state: Arc<ContextState>,
}

impl McrxContext {
    fn new() -> Self {
        Self {
            state: Arc::new(ContextState {
                context: Mutex::new(Context::new()),
                last_error: Mutex::new(None),
                active_worker_generation: AtomicU64::new(0),
                next_worker_generation: AtomicU64::new(1),
                worker: Mutex::new(None),
                idle_wait: Mutex::new(()),
                wake_worker: Condvar::new(),
            }),
        }
    }

    fn clear_error(&self) {
        if let Ok(mut last_error) = self.state.last_error.lock() {
            *last_error = None;
        }
    }

    fn set_error(&self, message: impl Into<String>) -> c_int {
        if let Ok(mut last_error) = self.state.last_error.lock() {
            *last_error = Some(cstring_lossy(message.into()));
        }
        MCRX_STATUS_ERROR
    }

    fn set_invalid_argument(&self, message: impl Into<String>) -> c_int {
        if let Ok(mut last_error) = self.state.last_error.lock() {
            *last_error = Some(cstring_lossy(message.into()));
        }
        MCRX_STATUS_INVALID_ARGUMENT
    }

    fn stop_worker(&self) -> Result<(), String> {
        self.state
            .active_worker_generation
            .store(0, Ordering::Release);
        self.state.wake_worker.notify_all();

        let mut worker = self
            .state
            .worker
            .lock()
            .map_err(|_| "mcrx worker mutex is poisoned".to_string())?;

        let Some(handle) = worker.take() else {
            return Ok(());
        };

        drop(worker);

        if handle.thread().id() == thread::current().id() {
            // Detach the current handle. Its generation was invalidated above,
            // so it must exit after the callback even if a replacement starts.
            return Ok(());
        }

        handle
            .join()
            .map_err(|_| "mcrx receive worker panicked".to_string())
    }
}

fn wait_for_worker_wakeup(
    state: &ContextState,
    generation: u64,
    timeout: Duration,
) -> Result<(), String> {
    let guard = state
        .idle_wait
        .lock()
        .map_err(|_| "mcrx worker wait mutex is poisoned".to_string())?;

    if state.active_worker_generation.load(Ordering::Acquire) != generation {
        return Ok(());
    }

    let (_guard, _timeout) = state
        .wake_worker
        .wait_timeout_while(guard, timeout, |_| {
            state.active_worker_generation.load(Ordering::Acquire) == generation
        })
        .map_err(|_| "mcrx worker wait mutex is poisoned".to_string())?;
    Ok(())
}

fn stop_worker_generation_with_error(state: &ContextState, generation: u64, error: String) {
    if state
        .active_worker_generation
        .compare_exchange(generation, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        if let Ok(mut last_error) = state.last_error.lock() {
            *last_error = Some(cstring_lossy(error));
        }
        state.wake_worker.notify_all();
    }
}

fn cstring_lossy(message: impl Into<String>) -> CString {
    match CString::new(message.into()) {
        Ok(message) => message,
        Err(_) => CString::new("mcrx error contained an interior nul byte")
            .expect("static string has no interior nul bytes"),
    }
}

fn set_last_invalid_argument(message: impl Into<String>) -> c_int {
    LAST_ERROR.with(|last_error| {
        *last_error.borrow_mut() = Some(cstring_lossy(message.into()));
    });
    MCRX_STATUS_INVALID_ARGUMENT
}

fn clear_last_error() {
    LAST_ERROR.with(|last_error| {
        *last_error.borrow_mut() = None;
    });
}

fn ffi_status(body: impl FnOnce() -> c_int) -> c_int {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(status) => status,
        Err(_) => {
            set_last_error_with_status("panic crossed mcrx-core-ffi boundary", MCRX_STATUS_PANIC)
        }
    }
}

fn set_last_error_with_status(message: impl Into<String>, status: c_int) -> c_int {
    LAST_ERROR.with(|last_error| {
        *last_error.borrow_mut() = Some(cstring_lossy(message.into()));
    });
    status
}

unsafe fn context_from_mut_ptr<'a>(context: *mut McrxContext) -> Result<&'a McrxContext, c_int> {
    if context.is_null() {
        return Err(set_last_invalid_argument(
            "context pointer must not be null",
        ));
    }

    Ok(unsafe { &*context })
}

unsafe fn context_from_const_ptr<'a>(
    context: *const McrxContext,
) -> Result<&'a McrxContext, c_int> {
    if context.is_null() {
        return Err(set_last_invalid_argument(
            "context pointer must not be null",
        ));
    }

    Ok(unsafe { &*context })
}

unsafe fn required_str<'a>(raw: *const c_char, field: &'static str) -> Result<&'a str, String> {
    if raw.is_null() {
        return Err(format!("{field} must not be null"));
    }

    unsafe { CStr::from_ptr(raw) }
        .to_str()
        .map_err(|_| format!("{field} must be valid UTF-8"))
}

unsafe fn optional_str<'a>(
    raw: *const c_char,
    field: &'static str,
) -> Result<Option<&'a str>, String> {
    if raw.is_null() {
        return Ok(None);
    }

    unsafe { required_str(raw, field) }.map(Some)
}

fn parse_ip_addr(raw: &str, field: &'static str) -> Result<IpAddr, String> {
    raw.parse()
        .map_err(|_| format!("invalid {field} IP address: {raw}"))
}

fn parse_optional_ip_addr(
    raw: Option<&str>,
    field: &'static str,
) -> Result<Option<IpAddr>, String> {
    raw.map(|value| parse_ip_addr(value, field)).transpose()
}

fn parse_interface_scope(raw: &str) -> Result<u32, String> {
    if raw.chars().all(|ch| ch.is_ascii_digit()) {
        let scope = raw
            .parse::<u32>()
            .map_err(|_| format!("invalid interface scope index: {raw}"))?;
        if scope == 0 {
            return Err("interface scope index must not be 0".to_string());
        }
        return Ok(scope);
    }

    interface_name_to_index(raw)
}

fn interface_name_to_index(raw: &str) -> Result<u32, String> {
    let raw = CString::new(raw).map_err(|_| "interface name must not contain NUL bytes")?;

    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::NetworkManagement::IpHelper::if_nametoindex;

        let index = if_nametoindex(raw.as_ptr().cast());
        if index == 0 {
            return Err(format!("unknown interface name: {}", raw.to_string_lossy()));
        }
        Ok(index)
    }

    #[cfg(not(windows))]
    unsafe {
        let index = libc::if_nametoindex(raw.as_ptr());
        if index == 0 {
            return Err(format!("unknown interface name: {}", raw.to_string_lossy()));
        }
        Ok(index)
    }
}

fn parse_interface_selector(
    group: IpAddr,
    raw: Option<&str>,
) -> Result<(Option<IpAddr>, Option<u32>), String> {
    let Some(raw) = raw else {
        return Ok((None, None));
    };

    if group.is_ipv6() {
        if let Some((addr, scope)) = raw.rsplit_once('%') {
            let addr = addr
                .parse::<Ipv6Addr>()
                .map_err(|_| format!("invalid interface IP address: {raw}"))?;
            let scope = parse_interface_scope(scope)?;
            return Ok((Some(IpAddr::V6(addr)), Some(scope)));
        }

        if raw.chars().all(|ch| ch.is_ascii_digit()) {
            let scope = parse_interface_scope(raw)?;
            return Ok((None, Some(scope)));
        }
    }

    Ok((Some(parse_ip_addr(raw, "interface")?), None))
}

fn build_subscription_config(
    group: &str,
    dst_port: u16,
    source: Option<&str>,
    interface: Option<&str>,
) -> Result<SubscriptionConfig, String> {
    let group = parse_ip_addr(group, "group")?;
    let source_addr = parse_optional_ip_addr(source, "source")?;
    let (interface_addr, interface_index) = parse_interface_selector(group, interface)?;

    let mut config = match source_addr {
        Some(source_addr) => SubscriptionConfig::ssm_ip(group, source_addr, dst_port),
        None => SubscriptionConfig::asm_ip(group, dst_port),
    };
    config.interface = interface_addr;
    config.interface_index = interface_index;

    Ok(config)
}

fn lock_context<'a>(state: &'a ContextState) -> Result<std::sync::MutexGuard<'a, Context>, String> {
    state
        .context
        .lock()
        .map_err(|_| "mcrx context mutex is poisoned".to_string())
}

fn receive_one(state: &ContextState) -> Result<Option<PacketWithMetadata>, String> {
    let mut context = lock_context(state)?;
    context
        .try_recv_any_with_metadata()
        .map_err(|err| err.to_string())
}

fn status_from_mcrx_result<T>(context: &McrxContext, result: Result<T, McrxError>) -> c_int {
    match result {
        Ok(_) => {
            context.clear_error();
            MCRX_STATUS_OK
        }
        Err(err) => context.set_error(err.to_string()),
    }
}

fn opt_ip_cstring(addr: Option<IpAddr>) -> Option<CString> {
    addr.map(|addr| cstring_lossy(addr.to_string()))
}

fn opt_socket_ip_cstring(addr: Option<SocketAddr>) -> Option<CString> {
    addr.map(|addr| cstring_lossy(addr.ip().to_string()))
}

fn opt_cstr_ptr(value: &Option<CString>) -> *const c_char {
    value.as_ref().map_or(ptr::null(), |value| value.as_ptr())
}

fn socket_port(addr: Option<SocketAddr>) -> u16 {
    addr.map_or(0, |addr| addr.port())
}

fn call_packet_callback(
    packet: &PacketWithMetadata,
    callback: McrxPacketCallback,
    user_data: *mut c_void,
) {
    let source_ip = cstring_lossy(packet.packet.source.ip().to_string());
    let group_ip = cstring_lossy(packet.packet.group.to_string());
    let socket_local_ip = opt_socket_ip_cstring(packet.metadata.socket_local_addr);
    let configured_interface_ip = opt_ip_cstring(packet.metadata.configured_interface);
    let destination_local_ip = opt_ip_cstring(packet.metadata.destination_local_ip);

    let view = packet_view(
        packet,
        &source_ip,
        &group_ip,
        &socket_local_ip,
        &configured_interface_ip,
        &destination_local_ip,
    );

    unsafe {
        callback(&view, user_data);
    }
}

fn packet_view(
    packet: &PacketWithMetadata,
    source_ip: &CString,
    group_ip: &CString,
    socket_local_ip: &Option<CString>,
    configured_interface_ip: &Option<CString>,
    destination_local_ip: &Option<CString>,
) -> McrxPacketView {
    let metadata: &ReceiveMetadata = &packet.metadata;

    McrxPacketView {
        payload: packet.packet.payload.as_ptr(),
        payload_len: packet.packet.payload.len(),
        subscription_id: packet.packet.subscription_id.0,
        source_ip: source_ip.as_ptr(),
        source_port: packet.packet.source.port(),
        group_ip: group_ip.as_ptr(),
        dst_port: packet.packet.dst_port,
        socket_local_ip: opt_cstr_ptr(socket_local_ip),
        socket_local_port: socket_port(metadata.socket_local_addr),
        configured_interface_ip: opt_cstr_ptr(configured_interface_ip),
        configured_interface_index: metadata.configured_interface_index.unwrap_or(0),
        has_configured_interface_index: metadata.configured_interface_index.is_some() as u8,
        destination_local_ip: opt_cstr_ptr(destination_local_ip),
        ingress_interface_index: metadata.ingress_interface_index.unwrap_or(0),
        has_ingress_interface_index: metadata.ingress_interface_index.is_some() as u8,
    }
}

fn worker_loop(
    state: Arc<ContextState>,
    generation: u64,
    callback: McrxPacketCallback,
    user_data: usize,
    idle_sleep: Duration,
) {
    while state.active_worker_generation.load(Ordering::Acquire) == generation {
        match receive_one(&state) {
            Ok(Some(packet)) => {
                call_packet_callback(&packet, callback, user_data as *mut c_void);
            }
            Ok(None) => {
                if let Err(err) = wait_for_worker_wakeup(&state, generation, idle_sleep) {
                    stop_worker_generation_with_error(&state, generation, err);
                    break;
                }
            }
            Err(err) => {
                stop_worker_generation_with_error(&state, generation, err);
                break;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mcrx_ffi_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn mcrx_last_error() -> *const c_char {
    LAST_ERROR.with(|last_error| {
        last_error
            .borrow()
            .as_ref()
            .map_or(ptr::null(), |message| message.as_ptr())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn mcrx_context_new() -> *mut McrxContext {
    match catch_unwind(|| {
        clear_last_error();
        Box::into_raw(Box::new(McrxContext::new()))
    }) {
        Ok(context) => context,
        Err(_) => {
            set_last_error_with_status("panic crossed mcrx-core-ffi boundary", MCRX_STATUS_PANIC);
            ptr::null_mut()
        }
    }
}

/// Frees a context created by `mcrx_context_new`.
///
/// # Safety
///
/// `context` must be either null or a pointer returned by `mcrx_context_new`
/// that has not already been freed. After this call returns, the pointer must
/// not be used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mcrx_context_free(context: *mut McrxContext) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if context.is_null() {
            return;
        }

        let context = unsafe { Box::from_raw(context) };
        let _ = context.stop_worker();
    }));
}

/// Returns the last context-local error message, or null if none is available.
///
/// # Safety
///
/// `context` must be either null or a valid `McrxContext` pointer. The returned
/// pointer is borrowed and remains valid only until the next FFI call that may
/// update the context error state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mcrx_context_last_error(context: *const McrxContext) -> *const c_char {
    match catch_unwind(AssertUnwindSafe(|| {
        let Ok(context) = (unsafe { context_from_const_ptr(context) }) else {
            return mcrx_last_error();
        };

        context
            .state
            .last_error
            .lock()
            .ok()
            .and_then(|last_error| last_error.as_ref().map(|message| message.as_ptr()))
            .unwrap_or(ptr::null())
    })) {
        Ok(message) => message,
        Err(_) => {
            set_last_error_with_status("panic crossed mcrx-core-ffi boundary", MCRX_STATUS_PANIC);
            mcrx_last_error()
        }
    }
}

/// Returns the number of subscriptions stored in the context.
///
/// # Safety
///
/// `context` must be either null or a valid `McrxContext` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mcrx_context_subscription_count(context: *const McrxContext) -> usize {
    match catch_unwind(AssertUnwindSafe(|| {
        let Ok(context) = (unsafe { context_from_const_ptr(context) }) else {
            return 0;
        };

        lock_context(&context.state).map_or(0, |context| context.subscription_count())
    })) {
        Ok(count) => count,
        Err(_) => {
            set_last_error_with_status("panic crossed mcrx-core-ffi boundary", MCRX_STATUS_PANIC);
            0
        }
    }
}

/// Adds a UDP multicast subscription to the context.
///
/// # Safety
///
/// `context` must be a valid `McrxContext` pointer. `group` must point to a
/// valid null-terminated UTF-8 string. `source` and `interface` may be null or
/// valid null-terminated UTF-8 strings. `subscription_id_out` must point to
/// writable memory for one `uint64_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mcrx_context_add_subscription(
    context: *mut McrxContext,
    group: *const c_char,
    dst_port: u16,
    source: *const c_char,
    interface: *const c_char,
    subscription_id_out: *mut u64,
) -> c_int {
    ffi_status(|| {
        let context = match unsafe { context_from_mut_ptr(context) } {
            Ok(context) => context,
            Err(status) => return status,
        };

        if subscription_id_out.is_null() {
            return context.set_invalid_argument("subscription_id_out must not be null");
        }

        let group = match unsafe { required_str(group, "group") } {
            Ok(group) => group,
            Err(err) => return context.set_invalid_argument(err),
        };
        let source = match unsafe { optional_str(source, "source") } {
            Ok(source) => source,
            Err(err) => return context.set_invalid_argument(err),
        };
        let interface = match unsafe { optional_str(interface, "interface") } {
            Ok(interface) => interface,
            Err(err) => return context.set_invalid_argument(err),
        };

        let config = match build_subscription_config(group, dst_port, source, interface) {
            Ok(config) => config,
            Err(err) => return context.set_invalid_argument(err),
        };

        let mut inner = match lock_context(&context.state) {
            Ok(inner) => inner,
            Err(err) => return context.set_error(err),
        };

        match inner.add_subscription(config) {
            Ok(id) => {
                unsafe {
                    *subscription_id_out = id.0;
                }
                context.clear_error();
                MCRX_STATUS_OK
            }
            Err(err) => context.set_error(err.to_string()),
        }
    })
}

/// Joins the multicast group for an existing subscription.
///
/// # Safety
///
/// `context` must be a valid `McrxContext` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mcrx_context_join_subscription(
    context: *mut McrxContext,
    subscription_id: u64,
) -> c_int {
    ffi_status(|| {
        let context = match unsafe { context_from_mut_ptr(context) } {
            Ok(context) => context,
            Err(status) => return status,
        };

        let mut inner = match lock_context(&context.state) {
            Ok(inner) => inner,
            Err(err) => return context.set_error(err),
        };

        status_from_mcrx_result(
            context,
            inner.join_subscription(mcrx_core::SubscriptionId(subscription_id)),
        )
    })
}

/// Leaves the multicast group for an existing subscription.
///
/// # Safety
///
/// `context` must be a valid `McrxContext` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mcrx_context_leave_subscription(
    context: *mut McrxContext,
    subscription_id: u64,
) -> c_int {
    ffi_status(|| {
        let context = match unsafe { context_from_mut_ptr(context) } {
            Ok(context) => context,
            Err(status) => return status,
        };

        let mut inner = match lock_context(&context.state) {
            Ok(inner) => inner,
            Err(err) => return context.set_error(err),
        };

        status_from_mcrx_result(
            context,
            inner.leave_subscription(mcrx_core::SubscriptionId(subscription_id)),
        )
    })
}

/// Removes a subscription from the context.
///
/// # Safety
///
/// `context` must be a valid `McrxContext` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mcrx_context_remove_subscription(
    context: *mut McrxContext,
    subscription_id: u64,
) -> c_int {
    ffi_status(|| {
        let context = match unsafe { context_from_mut_ptr(context) } {
            Ok(context) => context,
            Err(status) => return status,
        };

        let mut inner = match lock_context(&context.state) {
            Ok(inner) => inner,
            Err(err) => return context.set_error(err),
        };

        if inner.remove_subscription(mcrx_core::SubscriptionId(subscription_id)) {
            context.clear_error();
            MCRX_STATUS_OK
        } else {
            context.set_error(McrxError::SubscriptionNotFound.to_string())
        }
    })
}

/// Polls for up to `max_packets` packets and invokes `callback` once per packet.
///
/// # Safety
///
/// `context` must be a valid `McrxContext` pointer. `callback` must be a valid
/// function pointer when `max_packets` is non-zero. `received_out` must point to
/// writable memory for one `size_t`. Packet view pointers passed to `callback`
/// are borrowed and valid only for the duration of that callback invocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mcrx_context_poll(
    context: *mut McrxContext,
    max_packets: usize,
    callback: Option<McrxPacketCallback>,
    user_data: *mut c_void,
    received_out: *mut usize,
) -> c_int {
    ffi_status(|| {
        let context = match unsafe { context_from_mut_ptr(context) } {
            Ok(context) => context,
            Err(status) => return status,
        };

        if received_out.is_null() {
            return context.set_invalid_argument("received_out must not be null");
        }

        unsafe {
            *received_out = 0;
        }

        let Some(callback) = callback else {
            if max_packets == 0 {
                context.clear_error();
                return MCRX_STATUS_OK;
            }
            return context
                .set_invalid_argument("callback must not be null when max_packets is non-zero");
        };

        let mut received = 0usize;

        for _ in 0..max_packets {
            let packet = match receive_one(&context.state) {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(err) => return context.set_error(err),
            };

            call_packet_callback(&packet, callback, user_data);
            received += 1;
        }

        unsafe {
            *received_out = received;
        }

        context.clear_error();
        MCRX_STATUS_OK
    })
}

/// Starts a background polling thread that invokes `callback` for received packets.
///
/// # Safety
///
/// `context` must be a valid `McrxContext` pointer. `callback` must be a valid
/// function pointer. `user_data` is passed through unchanged and must remain
/// valid for as long as the callback may use it. Packet view pointers passed to
/// `callback` are borrowed and valid only for the duration of that callback
/// invocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mcrx_context_start(
    context: *mut McrxContext,
    callback: Option<McrxPacketCallback>,
    user_data: *mut c_void,
    idle_sleep_ms: u32,
) -> c_int {
    ffi_status(|| {
        let context = match unsafe { context_from_mut_ptr(context) } {
            Ok(context) => context,
            Err(status) => return status,
        };

        let Some(callback) = callback else {
            return context.set_invalid_argument("callback must not be null");
        };

        let mut worker = match context.state.worker.lock() {
            Ok(worker) => worker,
            Err(_) => return context.set_error("mcrx worker mutex is poisoned"),
        };

        if let Some(handle) = worker.take() {
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                *worker = Some(handle);
                return MCRX_STATUS_ALREADY_RUNNING;
            }
        }

        let generation = context
            .state
            .next_worker_generation
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.wrapping_add(1).max(1))
            })
            .expect("worker generation update always succeeds");
        context
            .state
            .active_worker_generation
            .store(generation, Ordering::Release);

        let state = Arc::clone(&context.state);
        let user_data = user_data as usize;
        let sleep_ms = if idle_sleep_ms == 0 {
            DEFAULT_IDLE_SLEEP_MS
        } else {
            u64::from(idle_sleep_ms)
        };

        *worker = Some(thread::spawn(move || {
            worker_loop(
                state,
                generation,
                callback,
                user_data,
                Duration::from_millis(sleep_ms),
            );
        }));

        context.clear_error();
        MCRX_STATUS_OK
    })
}

/// Stops the background polling thread, if one is running.
///
/// # Safety
///
/// `context` must be a valid `McrxContext` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mcrx_context_stop(context: *mut McrxContext) -> c_int {
    ffi_status(|| {
        let context = match unsafe { context_from_mut_ptr(context) } {
            Ok(context) => context,
            Err(status) => return status,
        };

        match context.stop_worker() {
            Ok(()) => {
                context.clear_error();
                MCRX_STATUS_OK
            }
            Err(err) => context.set_error(err),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
    use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
    use std::time::Instant;

    fn cstring(value: &str) -> CString {
        CString::new(value).unwrap()
    }

    fn unused_udp_port_v4() -> u16 {
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    unsafe extern "C" fn count_packet(packet: *const McrxPacketView, user_data: *mut c_void) {
        assert!(!packet.is_null());
        let counter = unsafe { &*(user_data.cast::<AtomicUsize>()) };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    struct RestartCallbackState {
        context: usize,
        calls: AtomicUsize,
        stop_status: AtomicI32,
        start_status: AtomicI32,
    }

    unsafe extern "C" fn restart_from_callback(
        packet: *const McrxPacketView,
        user_data: *mut c_void,
    ) {
        assert!(!packet.is_null());
        let state = unsafe { &*(user_data.cast::<RestartCallbackState>()) };
        if state.calls.fetch_add(1, Ordering::AcqRel) == 0 {
            let context = state.context as *mut McrxContext;
            state
                .stop_status
                .store(unsafe { mcrx_context_stop(context) }, Ordering::Release);
            state.start_status.store(
                unsafe { mcrx_context_start(context, Some(restart_from_callback), user_data, 1) },
                Ordering::Release,
            );
        }
    }

    #[test]
    fn add_join_and_poll_receives_packet() {
        let context = mcrx_context_new();
        assert!(!context.is_null());

        let group = cstring("239.1.2.3");
        let port = unused_udp_port_v4();
        let mut subscription_id = 0u64;
        let add_status = unsafe {
            mcrx_context_add_subscription(
                context,
                group.as_ptr(),
                port,
                ptr::null(),
                ptr::null(),
                &mut subscription_id,
            )
        };
        assert_eq!(add_status, MCRX_STATUS_OK);
        assert_eq!(unsafe { mcrx_context_subscription_count(context) }, 1);

        let join_status = unsafe { mcrx_context_join_subscription(context, subscription_id) };
        assert_eq!(join_status, MCRX_STATUS_OK);

        let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        sender
            .send_to(
                b"ffi packet",
                SocketAddrV4::new(Ipv4Addr::new(239, 1, 2, 3), port),
            )
            .unwrap();

        let counter = AtomicUsize::new(0);
        let deadline = Instant::now() + Duration::from_secs(1);

        while counter.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            let mut received = 0usize;
            let poll_status = unsafe {
                mcrx_context_poll(
                    context,
                    8,
                    Some(count_packet),
                    (&counter as *const AtomicUsize).cast_mut().cast(),
                    &mut received,
                )
            };
            assert_eq!(poll_status, MCRX_STATUS_OK);
            if received == 0 {
                thread::sleep(Duration::from_millis(10));
            }
        }

        assert_eq!(counter.load(Ordering::Relaxed), 1);

        unsafe {
            mcrx_context_free(context);
        }
    }

    #[test]
    fn zero_packet_poll_does_not_require_a_callback() {
        let context = mcrx_context_new();
        assert!(!context.is_null());

        let mut received = usize::MAX;
        let status = unsafe { mcrx_context_poll(context, 0, None, ptr::null_mut(), &mut received) };

        assert_eq!(status, MCRX_STATUS_OK);
        assert_eq!(received, 0);
        unsafe { mcrx_context_free(context) };
    }

    #[test]
    fn invalid_group_reports_context_error() {
        let context = mcrx_context_new();
        assert!(!context.is_null());

        let group = cstring("127.0.0.1");
        let mut subscription_id = 0u64;
        let status = unsafe {
            mcrx_context_add_subscription(
                context,
                group.as_ptr(),
                5000,
                ptr::null(),
                ptr::null(),
                &mut subscription_id,
            )
        };

        assert_eq!(status, MCRX_STATUS_ERROR);
        let message = unsafe { CStr::from_ptr(mcrx_context_last_error(context)) };
        assert!(message.to_string_lossy().contains("multicast"));

        unsafe {
            mcrx_context_free(context);
        }
    }

    #[test]
    fn parses_ipv6_interface_index() {
        let config = build_subscription_config("ff1e::8000:1234", 5000, None, Some("7")).unwrap();

        assert_eq!(config.interface, None);
        assert_eq!(config.interface_index, Some(7));
    }

    #[test]
    fn parses_scoped_ipv6_interface() {
        let config =
            build_subscription_config("ff12::8000:1234", 5000, None, Some("fe80::1%7")).unwrap();

        assert_eq!(config.interface, Some("fe80::1".parse().unwrap()));
        assert_eq!(config.interface_index, Some(7));
    }

    #[test]
    fn stopping_worker_interrupts_long_idle_wait() {
        let context = mcrx_context_new();
        assert!(!context.is_null());

        let status =
            unsafe { mcrx_context_start(context, Some(count_packet), ptr::null_mut(), 60_000) };
        assert_eq!(status, MCRX_STATUS_OK);
        thread::sleep(Duration::from_millis(20));

        let started = Instant::now();
        assert_eq!(unsafe { mcrx_context_stop(context) }, MCRX_STATUS_OK);
        assert!(started.elapsed() < Duration::from_secs(1));

        unsafe { mcrx_context_free(context) };
    }

    #[test]
    fn callback_can_stop_and_restart_worker_without_overlap() {
        let context = mcrx_context_new();
        assert!(!context.is_null());
        let group_addr = Ipv4Addr::new(239, 1, 2, 6);
        let group = cstring(&group_addr.to_string());
        let port = unused_udp_port_v4();
        let mut subscription_id = 0u64;
        assert_eq!(
            unsafe {
                mcrx_context_add_subscription(
                    context,
                    group.as_ptr(),
                    port,
                    ptr::null(),
                    ptr::null(),
                    &mut subscription_id,
                )
            },
            MCRX_STATUS_OK
        );
        assert_eq!(
            unsafe { mcrx_context_join_subscription(context, subscription_id) },
            MCRX_STATUS_OK
        );

        let state = RestartCallbackState {
            context: context as usize,
            calls: AtomicUsize::new(0),
            stop_status: AtomicI32::new(-1),
            start_status: AtomicI32::new(-1),
        };
        assert_eq!(
            unsafe {
                mcrx_context_start(
                    context,
                    Some(restart_from_callback),
                    (&state as *const RestartCallbackState).cast_mut().cast(),
                    1,
                )
            },
            MCRX_STATUS_OK
        );

        let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let destination = SocketAddrV4::new(group_addr, port);
        sender.send_to(b"restart-1", destination).unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        while state.start_status.load(Ordering::Acquire) == -1 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.stop_status.load(Ordering::Acquire), MCRX_STATUS_OK);
        assert_eq!(state.start_status.load(Ordering::Acquire), MCRX_STATUS_OK);

        sender.send_to(b"restart-2", destination).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while state.calls.load(Ordering::Acquire) < 2 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.calls.load(Ordering::Acquire), 2);

        assert_eq!(unsafe { mcrx_context_stop(context) }, MCRX_STATUS_OK);
        unsafe { mcrx_context_free(context) };
    }
}
