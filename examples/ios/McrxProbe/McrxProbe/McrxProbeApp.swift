import SwiftUI

@main
struct McrxProbeApp: App {
    @StateObject private var receiver = McrxReceiver()

    var body: some Scene {
        WindowGroup {
            ContentView(receiver: receiver)
        }
    }
}
