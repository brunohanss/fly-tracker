# Fly Tracker

Rust workspace for an insect detection and tracking system. The repository currently contains starter crates. Detection, tracking, safety, and hardware integration are not implemented yet.

## Requirements

- Rust 1.90 or later, with Cargo, rustfmt, and Clippy.

## Validation

Run these commands from the repository root:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Run the starter application with `cargo run -p app`.

Read `AGENTS.md` before implementation. It defines the architecture, safety requirements, and milestones.

Commit `Cargo.lock` to keep application dependency versions reproducible. Git excludes build output, editor history, and local environment files.
