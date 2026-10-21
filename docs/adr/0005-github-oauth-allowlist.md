# 0005. GitHub OAuth with an invite allow-list

- Status: Accepted
- Date: 2026-10-19

## Context

The app is on the public internet but is for two specific people, both with
GitHub accounts. Storing passwords, sending emails or running an identity
provider would be more work and more risk than the app itself.

## Decision

Sign in with GitHub OAuth and accept only the logins listed in the
`ALLOWED_GITHUB_USERS` environment variable. No OAuth scopes are requested;
the app only reads the public profile. After login, the server issues its own
random session token in an `HttpOnly` cookie, stored in SQLite with an expiry.
A development login (`DEV_LOGIN=1`) exists for local work and end-to-end tests
and is refused unless the server listens on a loopback address.

## Consequences

- No credentials are stored; account security (2FA, recovery) is GitHub's.
- Inviting or removing someone is a configuration change and a container
  restart.
- GitHub logins can be renamed and later claimed by someone else. The security
  hardening in #17 pins the GitHub account id seen on first login and checks the
  allow-list on every request, so a stale or reassigned login cannot get in.
- Being offline from GitHub blocks new logins, not existing sessions.

## Alternatives considered

- **Passwords or magic links:** password storage or an email provider, plus
  recovery flows, for two users.
- **A hosted identity provider:** another dependency and account to manage for
  no extra benefit.
- **HTTP basic auth at the reverse proxy:** no per-user identity, which Ritmo
  needs for ownership.
