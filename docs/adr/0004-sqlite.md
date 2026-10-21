# 0004. SQLite as the only database

- Status: Accepted
- Date: 2026-10-19

## Context

Two users, a few boards and a few hundred tasks per year. The app runs as one
process on one small VPS, and nobody is paid to operate a database.

## Decision

Use SQLite in WAL mode through [sqlx](https://github.com/launchbadge/sqlx),
with migrations embedded in the binary and applied on start. Queries use sqlx's
compile-time checked macros, with the offline metadata committed in `.sqlx/` so
builds do not need a database. Whiteboards are stored as Loro snapshots (one
blob per board); Ritmo uses ordinary relational tables.

## Consequences

- Nothing to install, run or upgrade besides the app; the database is a file
  on a bind-mounted volume.
- WAL lets readers, including the backup job, work without blocking writers.
- Backups are a consistent file copy (`VACUUM INTO`) rather than a dump
  pipeline.
- A single writer at a time and a single host: fine for this load, but the app
  cannot be scaled horizontally without changing this decision.
- Changing a query requires regenerating `.sqlx/`; CI checks it is current.

## Alternatives considered

- **PostgreSQL:** more features and concurrency than needed, and another
  service to run, secure, upgrade and back up.
- **Storing boards as files on disk:** simpler for snapshots, but Ritmo needs
  relational queries and transactions anyway, and one store is easier to back
  up consistently.
