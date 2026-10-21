# 0002. Loro CRDT for the whiteboard

- Status: Accepted
- Date: 2026-10-19

## Context

Both people edit the same board during the meeting, often at the same time and
sometimes on the same shape. Edits must feel instant (no round trip before the
UI updates), survive a dropped connection, and converge to the same board on
every device. Ritmo has none of these needs: its writes are small, per-user and
rarely concurrent.

## Decision

Model each Cartazes board as a [Loro](https://loro.dev) document, used through
its Rust crate on both the server and the WASM client. Clients apply edits
locally and exchange incremental binary updates over a WebSocket. The server
keeps its own copy of each board to send snapshots to new participants,
validate updates and persist them. Use the CRDT for the whiteboard only; Ritmo
stays server-authoritative.

## Consequences

- Concurrent edits merge without locks, and edits made offline merge on
  reconnect.
- Imports are idempotent and order-independent, which keeps the relay simple:
  the server forwards bytes unchanged and a duplicate delivery is harmless. A
  property test checks that replicas converge for any order of operations.
- The server can validate an update by importing it into a fork of the
  document before applying it.
- Loro is a large dependency and dominates the WASM size. Some of its
  transitive dependencies are flagged as unmaintained, which cargo-deny is
  configured to report but not fail on.
- Document history grows over time; snapshot size is bounded by explicit limits.

## Alternatives considered

- **Server-authoritative, last-writer-wins per shape:** simpler, but every edit
  waits for the server, offline edits are lost or need custom merging, and
  concurrent edits to one shape silently drop one side.
- **Yjs or its Rust port Yrs:** mature and widely used. Loro was preferred for
  its native Rust API used directly on both sides, its movable tree and list
  types for future z-ordering and grouping, and its compact snapshot format.
- **Automerge:** also Rust-native and a reasonable fit; Loro's richer
  container types and the same reasons as above tipped the choice.
