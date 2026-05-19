import Foundation

final class McrxReceiver: ObservableObject {
    @Published var isRunning = false
    @Published var packetCount = 0
    @Published var lastSource = "-"
    @Published var lastPayloadPreview = "-"
    @Published var errorMessage: String?

    private var context: OpaquePointer?
    private var subscriptionId: UInt64 = 0

    deinit {
        stop()
        if let context {
            mcrx_context_free(context)
        }
    }

    func start(group: String, port: UInt16, source: String?, interface: String?) {
        stop()
        reset()

        guard let context = makeContext() else {
            return
        }

        var subscriptionId = UInt64(0)
        let addStatus = withOptionalCString(source) { sourcePtr in
            withOptionalCString(interface) { interfacePtr in
                group.withCString { groupPtr in
                    mcrx_context_add_subscription(
                        context,
                        groupPtr,
                        port,
                        sourcePtr,
                        interfacePtr,
                        &subscriptionId
                    )
                }
            }
        }

        guard addStatus == MCRX_STATUS_OK else {
            setError(from: context, fallback: "failed to add subscription")
            return
        }

        let joinStatus = mcrx_context_join_subscription(context, subscriptionId)
        guard joinStatus == MCRX_STATUS_OK else {
            setError(from: context, fallback: "failed to join subscription")
            return
        }

        self.subscriptionId = subscriptionId

        let callback: McrxPacketCallback = { packet, userData in
            guard let packet, let userData else {
                return
            }

            let receiver = Unmanaged<McrxReceiver>
                .fromOpaque(userData)
                .takeUnretainedValue()
            receiver.handle(packet: packet.pointee)
        }

        let startStatus = mcrx_context_start(
            context,
            callback,
            Unmanaged.passUnretained(self).toOpaque(),
            10
        )

        guard startStatus == MCRX_STATUS_OK else {
            setError(from: context, fallback: "failed to start receive loop")
            return
        }

        isRunning = true
    }

    func pollOnce() {
        guard let context else {
            errorMessage = "receiver is not initialized"
            return
        }

        let callback: McrxPacketCallback = { packet, userData in
            guard let packet, let userData else {
                return
            }

            let receiver = Unmanaged<McrxReceiver>
                .fromOpaque(userData)
                .takeUnretainedValue()
            receiver.handle(packet: packet.pointee)
        }

        var received = 0
        let status = mcrx_context_poll(
            context,
            16,
            callback,
            Unmanaged.passUnretained(self).toOpaque(),
            &received
        )

        if status != MCRX_STATUS_OK {
            setError(from: context, fallback: "poll failed")
        }
    }

    func stop() {
        guard let context else {
            isRunning = false
            return
        }

        _ = mcrx_context_stop(context)

        if subscriptionId != 0 {
            _ = mcrx_context_leave_subscription(context, subscriptionId)
            _ = mcrx_context_remove_subscription(context, subscriptionId)
            subscriptionId = 0
        }

        isRunning = false
    }

    private func makeContext() -> OpaquePointer? {
        if let context {
            return context
        }

        guard let context = mcrx_context_new() else {
            errorMessage = lastGlobalError() ?? "failed to create mcrx context"
            return nil
        }

        self.context = context
        return context
    }

    private func reset() {
        packetCount = 0
        lastSource = "-"
        lastPayloadPreview = "-"
        errorMessage = nil
    }

    private func handle(packet: McrxPacketView) {
        let sourceIp = packet.source_ip.map { String(cString: $0) } ?? "?"
        let payloadPreview = previewPayload(packet)

        DispatchQueue.main.async {
            self.packetCount += 1
            self.lastSource = "\(sourceIp):\(packet.source_port)"
            self.lastPayloadPreview = payloadPreview
            self.errorMessage = nil
        }
    }

    private func previewPayload(_ packet: McrxPacketView) -> String {
        guard let payload = packet.payload, packet.payload_len > 0 else {
            return "<empty>"
        }

        let count = min(packet.payload_len, 48)
        let data = Data(bytes: payload, count: count)

        if let text = String(data: data, encoding: .utf8), text.allSatisfy(\.isPrintableOrWhitespace) {
            return packet.payload_len > count ? "\(text)..." : text
        }

        let hex = data.map { String(format: "%02x", $0) }.joined(separator: " ")
        return packet.payload_len > count ? "\(hex) ..." : hex
    }

    private func setError(from context: OpaquePointer, fallback: String) {
        if let raw = mcrx_context_last_error(context) {
            errorMessage = String(cString: raw)
        } else {
            errorMessage = fallback
        }
    }

    private func lastGlobalError() -> String? {
        guard let raw = mcrx_last_error() else {
            return nil
        }
        return String(cString: raw)
    }
}

private func withOptionalCString<T>(
    _ value: String?,
    _ body: (UnsafePointer<CChar>?) -> T
) -> T {
    guard let value else {
        return body(nil)
    }

    return value.withCString { pointer in
        body(pointer)
    }
}

private extension Character {
    var isPrintableOrWhitespace: Bool {
        if isWhitespace {
            return true
        }

        return unicodeScalars.allSatisfy { scalar in
            scalar.value >= 0x20 && scalar.value != 0x7f
        }
    }
}
