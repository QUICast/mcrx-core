use mcrx_core::{
    Context, McrxError, Packet, PacketWithMetadata, ReceiveMetadata, SourceFilter,
    SubscriptionConfig, SubscriptionId, SubscriptionState,
};
use pyo3::exceptions::{PyLookupError, PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule};
use std::cell::RefCell;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;

#[derive(Debug, Clone)]
struct SharedContext {
    inner: Rc<RefCell<Context>>,
}

impl SharedContext {
    fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Context::new())),
        }
    }
}

fn invalid_argument(message: impl Into<String>) -> PyErr {
    PyValueError::new_err(message.into())
}

fn borrow_error(kind: &'static str) -> PyErr {
    PyRuntimeError::new_err(format!(
        "mcrx_core {kind} is already borrowed by another operation"
    ))
}

fn parse_ip_addr(raw: &str, field: &'static str) -> PyResult<IpAddr> {
    raw.parse()
        .map_err(|_| invalid_argument(format!("invalid {field} IP address: {raw}")))
}

fn parse_optional_ip_addr(raw: Option<&str>, field: &'static str) -> PyResult<Option<IpAddr>> {
    raw.map(|value| parse_ip_addr(value, field)).transpose()
}

fn build_subscription_config(
    group: &str,
    dst_port: u16,
    source: Option<&str>,
    interface: Option<&str>,
) -> PyResult<SubscriptionConfig> {
    let group = parse_ip_addr(group, "group")?;
    let source_addr = parse_optional_ip_addr(source, "source")?;
    let interface_addr = parse_optional_ip_addr(interface, "interface")?;

    let mut config = match source_addr {
        Some(source_addr) => SubscriptionConfig::ssm_ip(group, source_addr, dst_port),
        None => SubscriptionConfig::asm_ip(group, dst_port),
    };
    config.interface = interface_addr;

    Ok(config)
}

fn addr_to_tuple(addr: SocketAddr) -> (String, u16) {
    (addr.ip().to_string(), addr.port())
}

fn opt_addr_to_tuple(addr: Option<SocketAddr>) -> Option<(String, u16)> {
    addr.map(addr_to_tuple)
}

fn source_filter_string(filter: &SourceFilter) -> (&'static str, Option<String>) {
    match filter {
        SourceFilter::Any => ("asm", None),
        SourceFilter::Source(source) => ("ssm", Some(source.to_string())),
    }
}

fn subscription_state_name(state: SubscriptionState) -> &'static str {
    match state {
        SubscriptionState::Bound => "bound",
        SubscriptionState::Joined => "joined",
    }
}

fn mcrx_error_to_py(err: McrxError) -> PyErr {
    match err {
        McrxError::InvalidDestinationPort
        | McrxError::InvalidMulticastGroup
        | McrxError::InvalidSourceAddress
        | McrxError::SourceAddressFamilyMismatch
        | McrxError::InterfaceAddressFamilyMismatch => PyValueError::new_err(err.to_string()),
        McrxError::SubscriptionNotFound => PyLookupError::new_err(err.to_string()),
        McrxError::SocketCreateFailed(io_err)
        | McrxError::SocketOptionFailed(io_err)
        | McrxError::SocketBindFailed(io_err)
        | McrxError::SocketLocalAddrFailed(io_err)
        | McrxError::SocketIoctlFailed(io_err)
        | McrxError::MulticastJoinFailed(io_err)
        | McrxError::MulticastLeaveFailed(io_err)
        | McrxError::ReceiveFailed(io_err)
        | McrxError::InterfaceProbeBindFailed(io_err)
        | McrxError::InterfaceProbeConnectFailed(io_err)
        | McrxError::InterfaceProbeLocalAddrFailed(io_err) => {
            PyOSError::new_err(io_err.to_string())
        }
        other => PyRuntimeError::new_err(other.to_string()),
    }
}

fn with_context<T>(
    shared: &SharedContext,
    f: impl FnOnce(&Context) -> PyResult<T>,
) -> PyResult<T> {
    let context = shared.inner.try_borrow().map_err(|_| borrow_error("context"))?;
    f(&context)
}

fn with_context_mut<T>(
    shared: &SharedContext,
    f: impl FnOnce(&mut Context) -> PyResult<T>,
) -> PyResult<T> {
    let mut context = shared
        .inner
        .try_borrow_mut()
        .map_err(|_| borrow_error("context"))?;
    f(&mut context)
}

fn context_packet_to_py(py: Python<'_>, packet: Packet) -> PyResult<Py<PyPacket>> {
    Py::new(py, PyPacket::from(packet))
}

fn context_packet_with_metadata_to_py(
    py: Python<'_>,
    packet: PacketWithMetadata,
) -> PyResult<Py<PyPacketWithMetadata>> {
    Py::new(py, PyPacketWithMetadata::from(packet))
}

#[pyclass(
    module = "mcrx_core._mcrx_core",
    name = "Context",
    unsendable,
    skip_from_py_object
)]
#[derive(Debug, Clone)]
struct PyContext {
    shared: SharedContext,
}

#[pymethods]
impl PyContext {
    #[new]
    fn new() -> Self {
        Self {
            shared: SharedContext::new(),
        }
    }

    fn subscription_count(&self) -> PyResult<usize> {
        with_context(&self.shared, |context| Ok(context.subscription_count()))
    }

    #[pyo3(signature = (group, dst_port, source=None, interface=None))]
    fn add_subscription(
        &self,
        py: Python<'_>,
        group: &str,
        dst_port: u16,
        source: Option<&str>,
        interface: Option<&str>,
    ) -> PyResult<Py<PySubscription>> {
        let config = build_subscription_config(group, dst_port, source, interface)?;
        let id = with_context_mut(&self.shared, |context| {
            context.add_subscription(config).map_err(mcrx_error_to_py)
        })?;

        Py::new(
            py,
            PySubscription {
                shared: self.shared.clone(),
                id,
            },
        )
    }

    fn get_subscription(
        &self,
        py: Python<'_>,
        subscription_id: u64,
    ) -> PyResult<Py<PySubscription>> {
        let id = SubscriptionId(subscription_id);
        with_context(&self.shared, |context| {
            if context.contains_subscription(id) {
                Ok(())
            } else {
                Err(PyLookupError::new_err(format!(
                    "mcrx_core subscription {subscription_id} not found"
                )))
            }
        })?;

        Py::new(
            py,
            PySubscription {
                shared: self.shared.clone(),
                id,
            },
        )
    }

    fn remove_subscription(&self, subscription_id: u64) -> PyResult<bool> {
        with_context_mut(&self.shared, |context| {
            Ok(context.remove_subscription(SubscriptionId(subscription_id)))
        })
    }

    fn recv_any_nowait(&self, py: Python<'_>) -> PyResult<Option<Py<PyPacket>>> {
        let packet = with_context_mut(&self.shared, |context| {
            context.try_recv_any().map_err(mcrx_error_to_py)
        })?;

        packet.map(|packet| context_packet_to_py(py, packet)).transpose()
    }

    fn recv_any_with_metadata_nowait(
        &self,
        py: Python<'_>,
    ) -> PyResult<Option<Py<PyPacketWithMetadata>>> {
        let packet = with_context_mut(&self.shared, |context| {
            context
                .try_recv_any_with_metadata()
                .map_err(mcrx_error_to_py)
        })?;

        packet
            .map(|packet| context_packet_with_metadata_to_py(py, packet))
            .transpose()
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Context(subscription_count={})",
            self.subscription_count()?
        ))
    }
}

#[pyclass(
    module = "mcrx_core._mcrx_core",
    name = "Subscription",
    unsendable,
    skip_from_py_object
)]
#[derive(Debug, Clone)]
struct PySubscription {
    shared: SharedContext,
    id: SubscriptionId,
}

#[pymethods]
impl PySubscription {
    #[getter]
    fn id(&self) -> u64 {
        self.id.0
    }

    #[getter]
    fn group(&self) -> PyResult<String> {
        with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            Ok(subscription.config().group.to_string())
        })
    }

    #[getter]
    fn dst_port(&self) -> PyResult<u16> {
        with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            Ok(subscription.config().dst_port)
        })
    }

    #[getter]
    fn interface(&self) -> PyResult<Option<String>> {
        with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            Ok(subscription.config().interface.map(|ip| ip.to_string()))
        })
    }

    #[getter]
    fn join_mode(&self) -> PyResult<&'static str> {
        with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            Ok(source_filter_string(&subscription.config().source).0)
        })
    }

    #[getter]
    fn source(&self) -> PyResult<Option<String>> {
        with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            Ok(source_filter_string(&subscription.config().source).1)
        })
    }

    fn join(&self) -> PyResult<()> {
        with_context_mut(&self.shared, |context| {
            context.join_subscription(self.id).map_err(mcrx_error_to_py)
        })
    }

    fn leave(&self) -> PyResult<()> {
        with_context_mut(&self.shared, |context| {
            context.leave_subscription(self.id).map_err(mcrx_error_to_py)
        })
    }

    fn remove(&self) -> PyResult<bool> {
        with_context_mut(&self.shared, |context| Ok(context.remove_subscription(self.id)))
    }

    fn is_joined(&self) -> PyResult<bool> {
        with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            Ok(subscription.is_joined())
        })
    }

    fn state(&self) -> PyResult<&'static str> {
        with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            Ok(subscription_state_name(subscription.state()))
        })
    }

    fn local_addr(&self) -> PyResult<Option<(String, u16)>> {
        with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            let addr = subscription.local_addr().map_err(mcrx_error_to_py)?;
            Ok(Some(addr_to_tuple(addr)))
        })
    }

    fn recv_nowait(&self, py: Python<'_>) -> PyResult<Option<Py<PyPacket>>> {
        let packet = with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            subscription.try_recv().map_err(mcrx_error_to_py)
        })?;

        packet.map(|packet| context_packet_to_py(py, packet)).transpose()
    }

    fn recv_with_metadata_nowait(
        &self,
        py: Python<'_>,
    ) -> PyResult<Option<Py<PyPacketWithMetadata>>> {
        let packet = with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            subscription
                .try_recv_with_metadata()
                .map_err(mcrx_error_to_py)
        })?;

        packet
            .map(|packet| context_packet_with_metadata_to_py(py, packet))
            .transpose()
    }

    #[cfg(unix)]
    fn fileno(&self) -> PyResult<i32> {
        with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            Ok(subscription.as_raw_fd())
        })
    }

    #[cfg(windows)]
    fn socket_handle(&self) -> PyResult<usize> {
        with_context(&self.shared, |context| {
            let subscription = context
                .get_subscription(self.id)
                .ok_or_else(|| PyLookupError::new_err("mcrx_core subscription not found"))?;
            Ok(subscription.as_raw_socket() as usize)
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Subscription(id={}, group={:?}, port={}, state={:?})",
            self.id(),
            self.group()?,
            self.dst_port()?,
            self.state()?,
        ))
    }
}

#[pyclass(
    module = "mcrx_core._mcrx_core",
    name = "Packet",
    skip_from_py_object
)]
#[derive(Debug, Clone)]
struct PyPacket {
    subscription_id: u64,
    source_addr: String,
    source_port: u16,
    group: String,
    dst_port: u16,
    payload: Vec<u8>,
}

impl From<Packet> for PyPacket {
    fn from(packet: Packet) -> Self {
        Self {
            subscription_id: packet.subscription_id.0,
            source_addr: packet.source.ip().to_string(),
            source_port: packet.source.port(),
            group: packet.group.to_string(),
            dst_port: packet.dst_port,
            payload: packet.payload.to_vec(),
        }
    }
}

#[pymethods]
impl PyPacket {
    #[getter]
    fn subscription_id(&self) -> u64 {
        self.subscription_id
    }

    #[getter]
    fn source(&self) -> (String, u16) {
        (self.source_addr.clone(), self.source_port)
    }

    #[getter]
    fn source_addr(&self) -> &str {
        &self.source_addr
    }

    #[getter]
    fn source_port(&self) -> u16 {
        self.source_port
    }

    #[getter]
    fn group(&self) -> &str {
        &self.group
    }

    #[getter]
    fn dst_port(&self) -> u16 {
        self.dst_port
    }

    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.payload)
    }

    fn payload_len(&self) -> usize {
        self.payload.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "Packet(subscription_id={}, source=({}, {}), group={:?}, dst_port={}, payload_len={})",
            self.subscription_id,
            self.source_addr,
            self.source_port,
            self.group,
            self.dst_port,
            self.payload.len(),
        )
    }
}

#[pyclass(
    module = "mcrx_core._mcrx_core",
    name = "ReceiveMetadata",
    skip_from_py_object
)]
#[derive(Debug, Clone)]
struct PyReceiveMetadata {
    socket_local_addr: Option<(String, u16)>,
    configured_interface: Option<String>,
    destination_local_ip: Option<String>,
    ingress_interface_index: Option<u32>,
}

impl From<ReceiveMetadata> for PyReceiveMetadata {
    fn from(metadata: ReceiveMetadata) -> Self {
        Self {
            socket_local_addr: opt_addr_to_tuple(metadata.socket_local_addr),
            configured_interface: metadata.configured_interface.map(|ip| ip.to_string()),
            destination_local_ip: metadata.destination_local_ip.map(|ip| ip.to_string()),
            ingress_interface_index: metadata.ingress_interface_index,
        }
    }
}

#[pymethods]
impl PyReceiveMetadata {
    #[getter]
    fn socket_local_addr(&self) -> Option<(String, u16)> {
        self.socket_local_addr.clone()
    }

    #[getter]
    fn configured_interface(&self) -> Option<&str> {
        self.configured_interface.as_deref()
    }

    #[getter]
    fn destination_local_ip(&self) -> Option<&str> {
        self.destination_local_ip.as_deref()
    }

    #[getter]
    fn ingress_interface_index(&self) -> Option<u32> {
        self.ingress_interface_index
    }

    fn __repr__(&self) -> String {
        format!(
            "ReceiveMetadata(socket_local_addr={:?}, configured_interface={:?}, destination_local_ip={:?}, ingress_interface_index={:?})",
            self.socket_local_addr,
            self.configured_interface,
            self.destination_local_ip,
            self.ingress_interface_index,
        )
    }
}

#[pyclass(
    module = "mcrx_core._mcrx_core",
    name = "PacketWithMetadata",
    skip_from_py_object
)]
#[derive(Debug, Clone)]
struct PyPacketWithMetadata {
    packet: PyPacket,
    metadata: PyReceiveMetadata,
}

impl From<PacketWithMetadata> for PyPacketWithMetadata {
    fn from(packet: PacketWithMetadata) -> Self {
        Self {
            packet: packet.packet.into(),
            metadata: packet.metadata.into(),
        }
    }
}

#[pymethods]
impl PyPacketWithMetadata {
    #[getter]
    fn packet(&self, py: Python<'_>) -> PyResult<Py<PyPacket>> {
        Py::new(py, self.packet.clone())
    }

    #[getter]
    fn metadata(&self, py: Python<'_>) -> PyResult<Py<PyReceiveMetadata>> {
        Py::new(py, self.metadata.clone())
    }

    fn __repr__(&self) -> String {
        format!(
            "PacketWithMetadata(packet={}, metadata={})",
            self.packet.__repr__(),
            self.metadata.__repr__(),
        )
    }
}

#[pymodule]
fn _mcrx_core(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyContext>()?;
    module.add_class::<PySubscription>()?;
    module.add_class::<PyPacket>()?;
    module.add_class::<PyReceiveMetadata>()?;
    module.add_class::<PyPacketWithMetadata>()?;
    Ok(())
}
