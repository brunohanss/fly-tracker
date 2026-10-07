# Fly Tracker

Rust workspace for deterministic insect detection, tracking, prediction, virtual aiming, and safety regression. The replay pipeline and connected dashboard work without hardware. Physical output is disabled.

## Requirements

- Rust 1.90 or later, with Cargo, rustfmt, and Clippy.

## Validation

Run these commands from the repository root:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Run a complete synthetic replay with the dashboard:

```powershell
cargo run -p app -- --synthetic slow_fly
cargo run --release -p app -- --headless --synthetic slow_fly --report target/slow-fly.json
```

Keys `1` to `5` select screens; Space pauses; arrows step through paused replay; `m` selects the foreground mask; `?` opens help; `q` or Ctrl+C exits. The dashboard refreshes at approximately 15 Hz and writes logs to `fly-tracker.log`.

The Live screen shows a grayscale half-block image with observed (`O`), trajectory (`.`), predicted (`P`), issued virtual aim (`+`), and suppressed request (`x`) overlays. Press `g` for a basic ASCII image if your terminal does not support grayscale colors or block characters. The image keeps its aspect ratio with an assumed 1:2 terminal cell shape. Preview data is bounded to 160 by 120 pixels and uses the same frame as the track overlays. It is an observation view, not a physical output control.

Run `cargo run -p app -- --headless` for a noninteractive startup check. This exits after reporting that no pipeline is connected.

Supported sources are seeded synthetic sequences, versioned PGM image manifests, and monochrome Y4M video. All use the same downstream pipeline. Synthetic safety labels test lockout logic; they do not validate image-based human or animal detection. Disk replay defaults to an unavailable safety detector and suppresses aiming. Use `--fixture-safety` only for labelled regression fixtures.

Use [the repeated-image test](docs/repeated-image-test.md) to test stationary targets in PNG/JPEG images without hardware.

See [implementation status](docs/implementation-status.md) for commands, measurements, limitations, and pending hardware work. Use `cargo run -p app -- --help` for CLI options. Hardware is not selected. Live OV9281 acquisition, physical galvo output, automatic wall calibration, and physical alignment validation remain open. NanoDet image clearance validation and Raspberry Pi deployment remain open.

Use [the virtual aim demo](docs/replay-image-safety.md) to cycle through stationary targets. NanoDet image inference is now selectable in replay configuration. It supplies protected-class presence scores; low scores remain uncertain and do not authorize aiming.

Use [the dog transition fixtures](tests/fixtures/dog-transition/README.md) to test fixed flies with dog entry and exit. The three safety-control regressions run with `cargo test --workspace`. The optional `app/nanodet-model-tests` feature also checks real image inference with the installed runtime and weights.

Use [the supplied human transition fixtures](tests/fixtures/human-transition/README.md) to test nine fixed flies with human entry and exit. Four additional regressions run with `cargo test --workspace`; the same optional feature checks real NanoDet human detection.

Use [the edge-hand fixtures](tests/fixtures/hand-transition/README.md) to test suppression on the first hand frame and repeated single-frame hand entry. Four control regressions run with `cargo test --workspace`; the optional model test requires actual human detection on each hand frame.

All replay sources now use a [500 ms virtual aiming delay](docs/virtual-aim-delay.md). Tracking and safety continue during the wait. A lockout cancels the pending request; clear recovery starts a new wait.

Read `AGENTS.md` before implementation. It defines the architecture, safety requirements, and milestones.

Commit `Cargo.lock` to keep application dependency versions reproducible. Git excludes build output, editor history, and local environment files.
