# Contributing to `octet`

Thank you for contributing to `octet`! This repository adheres to the **Google Rust Style Guide** and uses automated Git hooks to keep the codebase clean.

## Local Setup

Enable the repository's Git hooks (`.githooks/`):

```bash
git config core.hooksPath .githooks
```

## Quality Checks

Before opening a pull request, ensure all checks pass:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo doc --no-deps
```

## Commit Message Convention

We follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` — A new feature or header decoder
- `fix:` — A bug fix
- `docs:` — Documentation or screenshot updates
- `refactor:` — Code restructuring without behavior change
- `test:` — Adding or updating tests
- `ci:` / `chore:` — Build, CI, or tooling updates
