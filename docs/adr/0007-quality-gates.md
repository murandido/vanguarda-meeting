# 0007. Quality gates enforced in CI

- Status: Accepted
- Date: 2026-10-19

## Context

The project is built by one developer with heavy use of an AI coding
assistant. Code is produced quickly, so correctness cannot depend on the
author remembering to run checks. Some risks are specific to this design:
CRDT convergence, compile-time SQL metadata drifting from the schema, WASM
size creeping up, and real-time behavior that only shows up with two browsers.

## Decision

Define numbered quality gates and run them on every pull request. All of them
except QG-11 are required status checks for merging into `main`:

| Gate | Check |
| --- | --- |
| QG-01 | `cargo fmt --check` |
| QG-02 | Clippy with `pedantic`, warnings as errors, native and `wasm32` |
| QG-03 | Unit, integration and doc tests |
| QG-04 | Property test: CRDT replicas converge for any order of operations |
| QG-05 | sqlx offline metadata matches queries and migrations |
| QG-06 | cargo-deny: advisories, licenses, bans, sources |
| QG-07 | WASM release build within 10% of the gzip size budget |
| QG-08 | Playwright end-to-end tests with two users in separate browser contexts |
| QG-09 | Line coverage may not drop more than 0.1 pp below `main` |
| QG-10 | Pull request title follows Conventional Commits |
| QG-11 | Benchmarks build (informational) |

CI compiles the workspace once per heavy job: `rust` runs QG-01..05 and QG-09
(tests and coverage in a single instrumented pass), and `web` runs QG-07 and
QG-08. A lightweight job per gate then publishes each result under its own
status check name, so branch protection sees one check per gate. Every gate
runs even when an earlier one failed, so a pull request shows all problems at
once. Pull requests that only touch documentation skip compilation; `main`
always runs everything. `deny` runs on its own without compiling, and QG-10 and
QG-11 live in separate workflows so the deploy never waits for benchmarks.

Formatting and Clippy also run locally as pre-commit hooks (lefthook).
Third-party GitHub Actions are pinned by commit SHA and kept current by
Dependabot. Pull requests are squash-merged, so the validated title becomes the
commit message on `main`.

## Consequences

- Every change on `main` has passed the same checks, whoever or whatever wrote
  it.
- The coverage gate compares against a baseline cached from the latest `main`
  run; if the cache has expired, the gate passes with a notice.
- The WASM budget has to be raised deliberately, in a reviewed change, when a
  feature justifies it.
- CI takes a few minutes per push. Only `main` writes build caches; pull
  requests restore from them instead of competing for cache storage.

## Alternatives considered

- **Local hooks only:** easy to skip and not visible in review.
- **A fixed coverage threshold:** arbitrary, and it either blocks early work or
  allows silent erosion. "Do not decrease" tracks the trend instead.
- **Fewer gates:** each gate covers a failure mode that is cheap to catch in CI
  and expensive to debug in production.
