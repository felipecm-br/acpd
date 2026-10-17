# Contributing to Waymux (`waymux` / `acpd`)

Thank you for contributing to **Waymux**! This guide covers development setup, quality gates, and pull request conventions.

## Development Setup

Requires Rust 1.85+ (edition 2024) and Tmux 3.2+.

```bash
# Clone the repository
git clone https://github.com/felipecm-br/acpd.git
cd acpd

# Build both waymux and acpd binaries
cargo build

# Run the test suite (30+ unit tests)
cargo test
```

See [AGENTS.md](AGENTS.md) for the complete module map and architectural invariants, and [README.md](README.md) for CLI and RPC usage.

## Quality Gates Before Submitting a Pull Request

Every commit and pull request must pass our automated CI suite with **zero warnings**:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

## Commit Best Practices

- **Atomic commits:** Each commit should represent a single logical change (one feature, fix, refactor, or documentation update).
- **Conventional commit messages:** Use clear prefixes (`feat(cockpit): ...`, `fix(hook): ...`, `docs: ...`) with a subject line under 72 characters.
- **Test coverage:** New logic—especially state transitions, `/proc` parsing, hook classification, and worktree operations—must include unit tests.

## Code Style & Performance Rules

- **Keep CLI subcommands synchronous:** Do not require the Tokio runtime for CLI commands routed in `src/lib.rs`.
- **No hardcoded user paths:** Always resolve runtime and config paths via `$XDG_RUNTIME_DIR`, `$XDG_CONFIG_HOME`, and `$HOME`.
- **English-only documentation:** All code comments, commit messages, scripts, and markdown files must be written in English.
