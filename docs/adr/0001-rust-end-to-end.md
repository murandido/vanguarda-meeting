# 0001. Rust end to end, with WASM only for the canvas engine

- Status: Accepted
- Date: 2026-10-19

## Context

Vanguarda has two features with different needs. Ritmo is forms and lists.
Cartazes is an interactive whiteboard that must stay at 60 fps and share its
data model with the server, which keeps its own copy of every board. The
project has one developer, so the number of languages, toolchains and build
pipelines matters as much as raw capability.

## Decision

Write the server, the shared protocol types and the whiteboard engine in Rust,
as one Cargo workspace. Compile the engine (`crates/canvas`) to WebAssembly for
the browser and natively for the server. Keep browser JavaScript to thin glue:
input events, the canvas render loop and the WebSocket. Do not use WASM for
anything else; the rest of the UI is server-rendered HTML ([ADR
0003](0003-htmx-server-rendered-ui.md)).

## Consequences

- The board model, its limits and the CRDT logic are one implementation, tested
  once with native Rust tests and used on both sides.
- Protocol types live in `crates/shared`, so a breaking change fails to compile
  instead of failing at runtime.
- One linter, formatter and test runner cover almost all of the code.
- The WASM module has a size cost (the CRDT library dominates it). CI enforces
  a size budget ([ADR 0007](0007-quality-gates.md)).
- Contributors need the `wasm32-unknown-unknown` target and `wasm-pack`.

## Alternatives considered

- **TypeScript front end with a Rust server:** a larger JS toolchain and the
  board model written twice, or shared through a generated binding.
- **TypeScript everywhere (Node server):** a single language too, but weaker
  guarantees for the long-running, stateful WebSocket rooms, and a heavier
  runtime on a small VPS.
- **A Rust UI framework compiled to WASM (Leptos, Yew, Dioxus) for every page:**
  a much larger client for pages that are just forms and lists.
