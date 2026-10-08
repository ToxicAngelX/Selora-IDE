# Phase Z protocol

`crates/selora-protocol` is the shared Rust DTO boundary for the native shell. It defines the protocol version, stable validated identifiers, workspace-relative paths, UTF-16 text positions, versioned selections, request/event envelopes, bootstrap state, and typed command/event payloads.

Every envelope carries protocol/workspace/core epoch metadata. Snapshot sequences are monotonic; stale sequence and sequence-gap conditions are represented as typed validation errors. The `core_instance_id` changes on a new Rust process, so a reconnect must request a fresh bootstrap before applying events. Approval identifiers are validated and consumed once. External URLs require absolute HTTPS with an explicit host and no userinfo. These checks keep path handling, URL opening, and approval state out of untrusted frontend code.

The frontend passes `protocolVersion` when requesting bootstrap. Unsupported commands return `unsupported_phase_slice` while later vertical slices are added behind feature flags. The protocol tests cover validation, ordering, stale/gap detection, approval consumption, and URL restrictions.
