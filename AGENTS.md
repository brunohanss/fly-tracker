# Fly Tracker — Implementation Project

You are implementing a high-quality real-time insect detection, tracking, trajectory-prediction, safety, and precision-aiming platform in Rust.

Read `README.md` and `AGENTS.md` completely before modifying the repository.

`AGENTS.md` is the engineering contract for this project. Follow it throughout implementation.

# Objective

Build a fixed wall-facing system capable of:

1. Observing a defined area with a high-frame-rate global-shutter camera.
2. Detecting small flying insects.
3. Maintaining persistent target tracks.
4. Estimating target velocity.
5. Predicting target position to compensate for system latency.
6. Mapping predicted camera coordinates to physical XY galvo coordinates.
7. Moving an XY galvanometer to the predicted position.
8. Validating targeting accuracy with a low-power Class 1/2 visible alignment laser.
9. Immediately entering a safety lockout if a human, dog, or cat is partially or fully visible in the monitored region.

The initial hardware target is:

- Raspberry Pi 4, 1 GB
- OV9281 monochrome global-shutter camera
- 20Kpps XY galvanometer scanner
- dual-channel SPI DAC
- analog galvo signal interface
- low-power Class 1/2 visible alignment laser
- fixed wall-facing installation

The system must remain usable without physical hardware through prerecorded and synthetic inputs.

# Safety invariant

Safety has priority over tracking and aiming.

The system MUST NOT issue an aiming/pulse output when:

- a human is partially or fully visible;
- a dog is partially or fully visible;
- a cat is partially or fully visible;
- the safety detector is unavailable;
- the safety detector reports an error;
- the latest safety result is stale;
- camera input is stale;
- safety confidence is insufficient;
- the system cannot establish that the monitored region is clear.

Use fail-closed behavior.

Represent this explicitly in the state machine with a `SafetyLockout` state.

A safety lockout must override all target-tracking states.

A target remaining visible during a safety lockout must not override the lockout.

Do not implement or optimize a hazardous or destructive emitter.

# Engineering priorities

Optimize in this order:

1. Safety.
2. Correctness.
3. Deterministic behavior.
4. Measurable low latency.
5. Low latency variance.
6. Testability.
7. Maintainability.
8. Resource efficiency.
9. Raw throughput.

Do not optimize based only on intuition.

Measure first.

# Architecture

Use a Cargo workspace.

Maintain clear subsystem boundaries:

- `core`
- `camera`
- `safety`
- `vision`
- `tracking`
- `aiming`
- `hardware`
- `telemetry`
- `app`

Dependency direction must remain clean.

Hardware code must not leak into tracking logic.

Safety must remain independent from insect detection.

Detection and tracking must remain separate concepts.

A `Detection` is one observation.

A `Track` is a target state estimated across time.

# Real-time behavior

Timestamp every frame as close to acquisition as possible with a monotonic clock.

Do not assume a constant frame interval.

Use measured timestamps for tracking and prediction.

Do not build an unbounded frame queue.

Prefer dropping stale frames over processing stale frames.

Measure at least:

- frame acquisition latency;
- safety inference latency;
- detection latency;
- tracking latency;
- prediction latency;
- aiming-command latency;
- complete frame-to-command latency.

Report:

- P50;
- P95;
- P99;
- maximum.

FPS alone is not an acceptable performance metric.

# Memory

Avoid allocation in the steady-state processing loop where practical.

Allocate reusable buffers during initialization.

Reuse image matrices, masks, detections, tracking structures, and telemetry buffers.

Avoid complete frame copies.

# Concurrency

Do not introduce Tokio unless a measured requirement justifies it.

Prefer synchronous code and dedicated threads.

Use bounded communication between stages.

Give each hardware resource one clear owner.

Avoid shared mutable state.

# Computer vision

Start with classical computer vision for insect detection.

Initial candidate pipeline:

Frame
→ background model
→ foreground difference
→ threshold
→ minimal morphology
→ connected components
→ geometry filtering
→ detections

Do not introduce a neural network for insect detection until measurements show that classical CV is insufficient.

The safety detector is independent and can use a different approach because its failure characteristics are different.

# Tracking

Start with a state model equivalent to:

[x, y, vx, vy]

Use timestamp-aware state updates.

Keep these concepts separate:

- measurement;
- association;
- filtering;
- prediction;
- track lifecycle.

Track lifecycle must explicitly represent:

- candidate;
- confirmed;
- temporarily lost;
- removed.

Evaluate prediction accuracy at:

- current position;
- +5 ms;
- +10 ms;
- +20 ms.

# Aiming

Define an `AimingDevice` abstraction.

Implement a virtual aimer before physical hardware.

The virtual aimer must render the requested aim point into debug output.

Physical galvo support must be an adapter behind the same abstraction.

Calibration must map camera coordinates to normalized aiming coordinates.

Do not hard-code calibration constants.

Calibration data must be serializable and versioned.

# Replay

Replayability is mandatory.

The same downstream pipeline must support:

- prerecorded video;
- deterministic image sequences;
- synthetic sequences;
- live OV9281 input.

A recorded failure must be reproducible without physical hardware.

# Synthetic fixtures

Generate deterministic synthetic sequences from:

- wall backgrounds;
- isolated fly sprites;
- distractor sprites;
- safety fixtures.

Every generated sequence must have ground-truth metadata.

Ground truth must include target coordinates for each frame.

Randomized generation must accept an explicit seed.

A failing randomized test must report its seed.

Create scenarios for:

- empty wall;
- slow fly;
- fast fly;
- abrupt acceleration;
- takeoff;
- landing;
- target leaving the frame;
- target re-entering;
- multiple flies;
- temporary occlusion;
- difficult background;
- dust/debris;
- moving non-target object.

Safety regression scenarios must include:

- full human;
- partially visible human;
- hand entering at frame edge;
- arm entering at frame edge;
- full dog;
- partially visible dog;
- full cat;
- partially visible cat;
- fly plus human;
- fly plus dog;
- fly plus cat.

For every safety-positive fixture:

EXPECTED OUTPUT = SAFETY LOCKOUT

No aiming/pulse command is permitted.

# Rust quality rules

Prefer explicit domain types over ambiguous primitives.

Examples:

- `FrameId`
- `TargetId`
- `PixelPosition`
- `PixelVelocity`
- `NormalizedAim`
- `GalvoPosition`
- `FrameTimestamp`
- `Confidence`

Do not use `(f32, f32)` interchangeably for different coordinate systems.

Represent system states with enums.

Make invalid states difficult to represent.

Use safe Rust by default.

Use `#![forbid(unsafe_code)]` where practical.

If FFI requires unsafe code, isolate it behind a small safe abstraction.

Every unsafe block must document its invariant.

Use `thiserror` for typed library errors.

Application composition can use `anyhow`.

Do not use `unwrap()` or `expect()` for recoverable runtime failures.

Use structured `tracing`.

Do not use `println!` as the production logging system.

# Testing

Use:

- unit tests;
- property tests;
- deterministic replay tests;
- regression tests;
- benchmarks;
- hardware-in-the-loop tests when hardware becomes available.

Coordinate transformations require round-trip tests where applicable.

Numerical code must test boundary values.

Unexpected NaN or infinity values are failures.

# Required validation

Before considering a milestone complete, run:

cargo fmt --check

cargo clippy --workspace --all-targets --all-features -- -D warnings

cargo test --workspace

Do not declare success when these checks fail.

# Milestones

Implement incrementally.

## M0 — Foundation

Workspace, CI, configuration, tracing, domain types, state machine, and test infrastructure.

## M1 — Replay

Implement prerecorded video and deterministic image-sequence `FrameSource` implementations.

## M2 — Safety framework

Implement `SafetyState`, `SafetyLockout`, watchdog behavior, stale-result handling, and safety test fixtures.

No physical aiming output is allowed yet.

## M3 — Insect detection

Implement classical moving-object detection and debug visualization.

## M4 — Tracking

Implement target association and persistent tracks.

## M5 — Prediction

Implement timestamp-aware trajectory estimation and prediction.

Measure prediction error.

## M6 — Virtual aiming

Connect predicted target positions to the virtual aimer.

Render target position, predicted position, and requested aim point.

Safety lockout must suppress the virtual aiming command.

## M7 — Telemetry and regression suite

Implement latency distributions, accuracy measurements, deterministic replay reports, and regression fixtures.

## M8 — OV9281

Integrate live camera input without changing downstream APIs.

## M9 — Galvo adapter

Implement the physical galvo output adapter.

Keep all hardware-specific behavior isolated.

Safety lockout must physically suppress output.

## M10 — Wall calibration

Implement automatic camera-to-galvo calibration against the controlled wall plane.

Calibration must be measurable and repeatable.

## M11 — Closed-loop alignment

Use only the low-power alignment laser.

Measure physical aiming error.

Validate safety lockout with the complete hardware pipeline.

# Working method

For each milestone:

1. Read the existing implementation.
2. Read existing tests.
3. Identify the smallest coherent change.
4. State the proposed implementation plan.
5. Implement it.
6. Add or update tests.
7. Run formatting.
8. Run Clippy.
9. Run tests.
10. Run relevant benchmarks.
11. Report measured results.
12. Document known limitations.

Do not proceed blindly through multiple milestones.

Keep each milestone independently testable.

Do not replace a simple, measurable solution with a more complex solution unless evidence justifies the change.