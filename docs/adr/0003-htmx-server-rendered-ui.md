# 0003. Server-rendered UI with HTMX

- Status: Accepted
- Date: 2026-10-19

## Context

Apart from the whiteboard canvas, the UI is lists, forms and a Kanban board
that must work well on a phone for the daily check-in and update live while
both people look at it. One developer maintains everything.

## Decision

Render pages and fragments on the server with [Askama](https://github.com/askama-rs/askama)
templates and use [HTMX](https://htmx.org) for partial updates. Live updates
use Server-Sent Events that tell the page to re-fetch a fragment, instead of
pushing data to a client-side store. The only client-side application is the
canvas engine ([ADR 0001](0001-rust-end-to-end.md)).

## Consequences

- No front-end build step, bundler or client state management for most of the
  app. HTMX is vendored as one static file.
- Templates are compiled and type-checked with the Rust code, and escape HTML
  by default.
- State lives in one place (the server), which makes authorization and
  validation straightforward to test with plain HTTP integration tests.
- Each interaction costs a round trip. That is acceptable for forms; the latency
  sensitive part (the canvas) is local-first.
- Richer interactions such as drag and drop on the Kanban board need small,
  targeted scripts; for now cards move with buttons.

## Alternatives considered

- **React or Svelte single-page app:** a second language and toolchain, an API
  layer to design and secure, and duplicated validation, for little benefit on
  form-driven pages.
- **A Rust WASM UI framework (Leptos, Dioxus):** one language, but a large WASM
  bundle on every page and a less mature ecosystem for this kind of app.
