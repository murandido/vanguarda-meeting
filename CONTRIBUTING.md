# Contributing

Vanguarda is a personal project with a single maintainer, built for two
specific users. Issues and suggestions are welcome; for anything larger than a
small fix, please open an issue before writing code.

## Development setup

Follow [Getting started](README.md#getting-started) in the README. In short:

```sh
lefthook install                                     # optional pre-commit hooks
wasm-pack build crates/canvas --target web --release
DEV_LOGIN=1 ALLOWED_GITHUB_USERS=ana,bia cargo run -p server
```

## Before opening a pull request

CI runs every [quality gate](README.md#testing-and-quality-gates); running the
quick ones locally saves a round trip:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Changed a SQL query or migration? Run `cargo sqlx prepare --workspace` and
  commit `.sqlx/`.
- Changed the canvas engine? Check the WASM size with
  `scripts/check-wasm-size.sh crates/canvas/pkg/canvas_bg.wasm`.
- Changed behavior a user can see? Add or update a Playwright test in `e2e/`.

## Conventions

- **Language:** code, comments, commits, pull requests and documentation are in
  English. The user interface is in Portuguese (Brazil), because that is what
  its users speak. Some older code comments are still in Portuguese and are
  being translated.
- **Commits and PR titles** follow [Conventional Commits](https://www.conventionalcommits.org)
  (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `ci:`, `chore:`, with an
  optional scope such as `feat(ritmo):`). Pull requests are squash-merged, so
  the title becomes the commit on `main`; CI checks it.
- Keep pull requests focused, with one concern per commit.
- Significant design changes come with an [ADR](docs/adr/).

## Pull request flow

1. Branch from `main`.
2. Open a pull request using the template: summary, changes, how it was tested.
3. All required CI checks must pass, and the pull request is reviewed before
   it is merged.
4. Merging to `main` deploys to production automatically.

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE).
