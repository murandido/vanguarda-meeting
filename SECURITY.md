# Security policy

## Supported versions

Only the `main` branch is supported. It is deployed continuously, so fixes ship
as soon as they are merged. There are no released versions.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's
[private vulnerability reporting](https://github.com/murandido/vanguarda-meeting/security/advisories/new)
("Report a vulnerability" on the Security tab). Do not open a public issue.

Include what you found, how to reproduce it and the impact you expect. You can
expect an acknowledgement within a week. This is a personal project, so there
is no bug bounty, but valid reports are credited in the advisory if you wish.

## Scope

In scope:

- the code in this repository: the server, the canvas engine, the deploy
  scripts and the CI/CD workflows;
- authentication and session handling, authorization between the two users,
  the WebSocket and SSE endpoints, and the container configuration.

Out of scope:

- the production instance itself: do not test against it, scan it or try to
  access it. Reproduce issues locally (see the README);
- vulnerabilities in dependencies that are already tracked upstream or in
  RustSec, unless this project uses them in an exploitable way;
- denial of service through traffic volume, and social engineering.

The security model and the controls in place are described in
[docs/architecture.md](docs/architecture.md#security-model).
