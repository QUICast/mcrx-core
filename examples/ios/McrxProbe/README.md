# McrxProbe

Minimal iOS SwiftUI test app for `mcrx-core-ffi`.

The app links the locally generated `McrxCore.xcframework` from the repository
root. Build that first:

```bash
cargo build -p mcrx-core-ffi --release --target aarch64-apple-ios
cargo build -p mcrx-core-ffi --release --target aarch64-apple-ios-sim

xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libmcrx_core_ffi.a -headers mcrx-core-ffi/include \
  -library target/aarch64-apple-ios-sim/release/libmcrx_core_ffi.a -headers mcrx-core-ffi/include \
  -output McrxCore.xcframework
```

Then open:

```bash
open examples/ios/McrxProbe/McrxProbe.xcodeproj
```

## Notes

- Simulator builds can validate linking and UI flow.
- Physical devices need `NSLocalNetworkUsageDescription`.
- Custom multicast/broadcast on physical devices requires Apple's multicast
  networking entitlement in the provisioning profile.
- The checked-in entitlement file contains
  `com.apple.developer.networking.multicast`; Xcode/device signing still needs
  an Apple-approved profile for that entitlement.
- The project excludes `x86_64` simulator builds because the default
  XCFramework command above builds the arm64 simulator target only. Add
  `x86_64-apple-ios-sim` to the Rust build if you need Intel simulator support.
