import SwiftUI

struct ContentView: View {
    @ObservedObject var receiver: McrxReceiver

    @State private var group = "239.1.2.3"
    @State private var port = "5000"
    @State private var source = ""
    @State private var interface = ""

    var body: some View {
        NavigationStack {
            Form {
                Section("Subscription") {
                    TextField("Group", text: $group)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Port", text: $port)
                        .keyboardType(.numberPad)
                    TextField("Source, optional", text: $source)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Interface, optional", text: $interface)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }

                Section("Receive") {
                    HStack {
                        Button(receiver.isRunning ? "Stop" : "Start") {
                            toggleReceiver()
                        }
                        .buttonStyle(.borderedProminent)

                        Button("Poll Once") {
                            receiver.pollOnce()
                        }
                        .disabled(receiver.isRunning)
                    }

                    LabeledContent("Packets", value: "\(receiver.packetCount)")
                    LabeledContent("Last source", value: receiver.lastSource)
                    LabeledContent("Last payload", value: receiver.lastPayloadPreview)
                }

                if let error = receiver.errorMessage {
                    Section("Error") {
                        Text(error)
                            .foregroundStyle(.red)
                            .font(.footnote)
                    }
                }
            }
            .navigationTitle("Mcrx Probe")
        }
    }

    private func toggleReceiver() {
        if receiver.isRunning {
            receiver.stop()
            return
        }

        guard let dstPort = UInt16(port) else {
            receiver.errorMessage = "Port must be between 0 and 65535"
            return
        }

        receiver.start(
            group: group,
            port: dstPort,
            source: source.nilIfEmpty,
            interface: interface.nilIfEmpty
        )
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
