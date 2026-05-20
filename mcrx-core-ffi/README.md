# mcrx-core-ffi

C ABI bindings for `mcrx-core`.

This crate is intended for Swift, C, C++, Kotlin/NDK, and other non-Rust
consumers that need the normal UDP multicast receiver API from `mcrx-core`.
The C surface is deliberately small and stable-looking:

- create and free a receiver context
- add, join, leave, and remove UDP multicast subscriptions
- poll explicitly from a host event loop
- optionally run a small background receive loop with a packet callback

Raw packet receive is not exposed in the first FFI pass. Keep using the Rust
`raw-packets` API directly until the C shape for complete IP datagrams is clear.

## Header

The public header is handwritten at:

```text
include/mcrx_core_ffi.h
```

Pointers in `McrxPacketView` are borrowed and are valid only for the duration of
the callback.

## iOS Build Sketch

For iOS, consume the `staticlib` output through an XCFramework:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

cargo build -p mcrx-core-ffi --release --target aarch64-apple-ios
cargo build -p mcrx-core-ffi --release --target aarch64-apple-ios-sim

xcodebuild -create-xcframework \
  -library ../target/aarch64-apple-ios/release/libmcrx_core_ffi.a -headers include \
  -library ../target/aarch64-apple-ios-sim/release/libmcrx_core_ffi.a -headers include \
  -output McrxCore.xcframework
```

The physical-device probe app lives in `examples/ios/McrxProbe` within this
crate and expects that local `McrxCore.xcframework`.

Physical iOS devices still need Apple local-network privacy configuration and,
for custom multicast/broadcast use, the multicast networking entitlement.
