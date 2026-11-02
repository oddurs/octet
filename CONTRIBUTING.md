# Contributing to `octet`

Thank you for contributing to `octet`! Formatting is `rustfmt` with the repository's `rustfmt.toml`; Git hooks run the same checks as CI.

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
cargo test
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

## Commit Message Convention

We follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` — A new feature or header decoder
- `fix:` — A bug fix
- `docs:` — Documentation or screenshot updates
- `refactor:` — Code restructuring without behavior change
- `test:` — Adding or updating tests
- `ci:` / `chore:` — Build, CI, or tooling updates

## License

Contributions are dual-licensed under MIT OR Apache-2.0, matching the project.
