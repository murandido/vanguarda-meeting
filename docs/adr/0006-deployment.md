# 0006. Cross-compiled arm64 image, GHCR and SSH deploy with rollback

- Status: Accepted
- Date: 2026-10-20

## Context

The app runs on an existing ARM (aarch64) VPS that already hosts other
services behind a shared Caddy reverse proxy. GitHub's standard runners are
x86_64. Building Rust for arm64 under QEMU emulation takes tens of minutes.
Every merge to `main` should reach production without manual steps, and a bad
release must not leave the site down.

## Decision

- **Build:** cross-compile the server for `aarch64-unknown-linux-gnu` on an
  x86_64 runner with the GNU cross toolchain. The Dockerfile only packages the
  prebuilt binary, static files and the WASM package (the same one that passed
  the end-to-end tests) on a slim Debian base, running as an unprivileged user.
- **Publish:** push the `linux/arm64` image to GHCR, tagged with the commit SHA.
- **Deploy:** after CI succeeds on `main`, copy the deploy files over SSH and run
  `deploy/deploy.sh`, which recreates the container with Docker Compose, waits
  for the container health check and redeploys the previous tag if it fails.
- **Proxy:** contribute one Caddy site file, rendered from a template, to the
  shared reverse proxy; reload Caddy only when the file changes and revert it if
  Caddy rejects it.
- **Configuration:** host-specific names (proxy network, Caddy container, sites
  directory, domain) live on the server and in GitHub environment variables, not
  in the repository.

## Consequences

- Image builds take minutes instead of tens of minutes, and Docker layers are
  cached in the Actions cache so the emulated `apt-get` layer rarely reruns.
- The same deploy installs and updates the daily SQLite backup timer, so backup
  configuration is versioned with the app.
- Deploys are serialized and only run for commits that passed every gate.
- A failed release is rolled back automatically; the previous image is kept
  locally for a fast rollback.
- The runtime image's glibc must be at least the build runner's; the Dockerfile
  documents the pairing.
- The deploy key has shell access to the VPS, so it is dedicated to this
  workflow and scoped to the `production` environment.
- Downtime during a deploy is a few seconds while the container restarts,
  which is acceptable for two users.

## Alternatives considered

- **Building inside Docker with QEMU:** simplest, but far too slow.
- **ARM GitHub runners:** would remove cross-compilation, but were not needed
  once cross-compiling worked.
- **Pull-based deploys (Watchtower or similar):** no health-gated rollback and
  harder to trace back to a CI run.
- **A PaaS:** extra cost and less control for an app that fits on an existing
  machine.
