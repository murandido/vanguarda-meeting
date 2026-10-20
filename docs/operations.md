# Operations

How Vanguarda runs in production and how to operate it. For the reasoning
behind the setup, see [ADR 0006](adr/0006-deployment.md).

## Overview

- One container (`vanguarda-meeting`) on an ARM (aarch64) VPS, managed with
  Docker Compose from `/opt/vanguarda`.
- A Caddy reverse proxy, shared with other sites on the same host, terminates
  TLS and forwards to the container over a shared Docker network. Vanguarda
  only contributes one site file to it.
- The SQLite database lives on the host in `/opt/vanguarda/data` and is mounted
  into the container at `/data`.

## Continuous deployment

The [Deploy workflow](../.github/workflows/deploy.yml) runs when CI succeeds on
a push to `main` (or manually):

1. Cross-compiles the server for `aarch64-unknown-linux-gnu` on an x86 runner
   (no QEMU emulation for the Rust build).
2. Reuses the WASM package that passed CI's end-to-end tests.
3. Builds a `linux/arm64` image from the [`Dockerfile`](../Dockerfile) and pushes
   it to `ghcr.io/murandido/vanguarda-meeting:<commit-sha>` (and `:latest`).
4. Copies `deploy/compose.yml`, `deploy/deploy.sh`, `deploy/site.caddy.tmpl`,
   `deploy/backup.sh`, `deploy/restore.sh` and `deploy/systemd/` to
   `/opt/vanguarda/` over SSH and runs `deploy.sh <commit-sha>`.
5. Polls `{PUBLIC_URL}/health` from the runner.

[`deploy.sh`](../deploy/deploy.sh) then:

1. Pulls the image and recreates the container with `docker compose up -d`.
2. Waits up to 60 s for the container's health check
   (`/app/server --healthcheck`, which calls `/health`).
3. On success, records the tag in `.deploy-state`. On failure, prints the
   container logs and redeploys the previous tag.
4. Renders the Caddy site from the template and reloads Caddy only if the file
   changed. If Caddy rejects it, the file is removed and Caddy is reloaded with
   its previous configuration.
5. Prunes old images, keeping the current and previous tags for fast rollback.
6. Installs or updates the backup timer (see [Backups and restore](#backups-and-restore)).

## One-time setup

### GitHub

In the `production` environment of the repository:

| Kind | Name | Value |
| --- | --- | --- |
| Secret | `VPS_HOST` | VPS address |
| Secret | `VPS_USER` | SSH user that runs deployments: in the `docker` group, with non-interactive `sudo` for installing and starting the backup systemd units |
| Secret | `VPS_SSH_KEY` | Private key dedicated to deployments |
| Secret | `VPS_KNOWN_HOSTS` | The VPS's `known_hosts` line(s), from `ssh-keyscan` |
| Variable | `PUBLIC_URL` | Public base URL, e.g. `https://vanguarda.example.com` |

A GitHub OAuth App is also needed, with the callback URL
`{PUBLIC_URL}/auth/github/callback`.

### VPS

```text
/opt/vanguarda/
├── .env              app configuration and secrets (mode 600)
├── deploy.env        host-specific deploy settings (not secret)
├── data/             SQLite database, owned by uid 10001 (the container user)
├── backups/          daily, weekly and pre-restore database copies
├── backup.env        optional offsite backup target
├── compose.yml       ┐
├── deploy.sh         │
├── site.caddy.tmpl   │ copied by every deployment
├── backup.sh         │
├── restore.sh        │
└── systemd/          ┘
```

`/opt/vanguarda/.env` is read by the container (`env_file`) and by Compose for
variable interpolation:

```sh
# App (see the configuration table in the README)
PUBLIC_URL=https://vanguarda.example.com
ALLOWED_GITHUB_USERS=alice,bob
GITHUB_CLIENT_ID=...
GITHUB_CLIENT_SECRET=...

# Compose: existing Docker network shared with the Caddy container
PROXY_NETWORK=proxy_network_name
```

`/opt/vanguarda/deploy.env` is sourced by `deploy.sh`:

```sh
SITE_DOMAIN=vanguarda.example.com     # hostname in the Caddy site block
CADDY_CONTAINER=caddy                 # running Caddy container to reload
SITES_DIR=/srv/caddy/sites            # host directory the Caddyfile imports
```

The Caddy container must import every file in `SITES_DIR` (for example, mount
it at `/etc/caddy/sites` and add `import /etc/caddy/sites/*.caddy` to the
Caddyfile) and must be attached to `PROXY_NETWORK`, so it can reach the app at
`vanguarda-meeting:3000`.

`PUBLIC_URL` must be identical in the GitHub variable and in `.env`: the OAuth
callback is built from it, and the Origin checks on WebSocket upgrades and
form posts (the latter added by #17) compare requests against it.

## Routine tasks

The current image tag is stored in `/opt/vanguarda/.deploy-state`. Commands
that recreate the container need it:

```sh
cd /opt/vanguarda
export IMAGE_TAG="$(. ./.deploy-state; echo "$CURRENT_TAG")"
```

### Invite someone

1. Add their GitHub login to `ALLOWED_GITHUB_USERS` in `/opt/vanguarda/.env`.
2. Recreate the container: `docker compose up -d` (with `IMAGE_TAG` exported as
   above).

### Remove someone

Remove the login from `ALLOWED_GITHUB_USERS` and recreate the container. Until
the security hardening in
[#17](https://github.com/murandido/vanguarda-meeting/pull/17) lands, existing
sessions stay valid until they expire (30 days), so also delete that login's
rows from the `sessions` table. With #17, the allow-list is checked on every
request and revoked sessions are purged automatically.

### Roll back by hand

```sh
/opt/vanguarda/deploy.sh <previous-commit-sha>
```

`PREVIOUS_TAG` in `.deploy-state` holds the last good tag. The image must still
exist locally or on GHCR.

### Logs and health

```sh
docker logs --tail 100 -f vanguarda-meeting
docker inspect -f '{{.State.Health.Status}}' vanguarda-meeting
curl -fsS "$PUBLIC_URL/health"
```

## Backups and restore

- **Schedule.** `deploy/backup.sh` runs daily at 04:30 (America/Sao_Paulo)
  from the `vanguarda-backup.timer` systemd unit (`deploy/systemd/`).
  `deploy.sh` installs the units only when they change, runs one validation
  backup right after installing them (a failure fails the deploy), and enables
  the timer. `Persistent=true` catches up after downtime.
- **Consistency.** The copy uses `VACUUM INTO` in a throwaway `sqlite3`
  container pinned by digest, running as uid 10001 with no network. In WAL mode
  it reads a single-transaction snapshot without stopping or blocking the app.
  The compressed file is decompressed again and checked with
  `PRAGMA integrity_check` before it replaces anything.
- **Retention.** Under `/opt/vanguarda/backups/` (owned by the deploy user,
  mode 700, files 600): `daily/vanguarda-YYYY-MM-DD.db.gz` keeps 14 days,
  `weekly/vanguarda-YYYY-Www.db.gz` keeps 8 ISO weeks (a hard link to the
  newest copy of each week), and `pre-restore/` keeps the last 10 copies taken
  by `restore.sh`.
- **Offsite (optional, off by default).** Set
  `BACKUP_OFFSITE=rclone:<remote:path>` or `BACKUP_OFFSITE=scp:<user@host:path>`
  in `/opt/vanguarda/backup.env` to also ship the daily copy off the VPS
  (retention there is up to the destination).

```sh
systemctl list-timers vanguarda-backup.timer      # next run
sudo systemctl start vanguarda-backup.service     # back up now
journalctl -u vanguarda-backup.service -n 50      # logs
ls -l /opt/vanguarda/backups/daily
```

To restore:

```sh
/opt/vanguarda/restore.sh /opt/vanguarda/backups/daily/vanguarda-YYYY-MM-DD.db.gz
```

The script verifies the backup, asks for confirmation (or `--yes`), copies the
live database to `backups/pre-restore/`, stops the container, swaps the
database (removing stale `-wal`/`-shm` files), starts it again and waits for
the health check. If the app does not become healthy, it prints the command to
go back to the `pre-restore` copy. A backup made by a newer version (with newer
migrations) cannot be restored under an older image.

To test both scripts locally, point them at any directory: paths and names are
configurable (`VANGUARDA_DIR`, `DATA_DIR`, `BACKUP_DIR`, `DB_USER`,
`APP_CONTAINER`; see the scripts' headers).
