# Changelog

## 0.3.0 - 2026-07-10

- Added the opt-in `raw-shared-capture` feature and Linux-only
  `SharedRawContext` for high-cardinality raw multicast receive. It shares one
  capture socket per resolved IP family/interface, maintains independent ASM
  and SSM memberships, and demultiplexes each complete IP datagram by group
  and source without scanning every logical subscription.
- Added bounded logical-subscription and pending-packet bookkeeping plus
  optional shared capture metrics for capture sockets, active memberships,
  received packets, unmatched packets, and demultiplex matches.
- Duplicate `SharedRawContext` configs now produce independent logical handles
  backed by one reference-counted kernel membership. Each captured datagram
  reports every matching handle, and leaving one duplicate does not disturb the
  others.
- Shared contexts cache resolved interface keys while matching subscriptions
  exist, avoiding repeated platform interface discovery during high-cardinality
  setup without retaining unbounded historical entries.
- macOS and Windows explicitly report shared raw capture as unsupported; their
  existing `raw-packets` behavior is unchanged.
- Hardened Unix and Windows ancillary metadata parsing by clamping kernel-
  reported control lengths to the supplied buffers and rejecting malformed
  Windows control-message chains that do not advance safely.
