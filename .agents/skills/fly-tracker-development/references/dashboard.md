# Ratatui dashboard requirements

Build an observability interface that makes the current system state understandable in approximately two seconds. Use Ratatui and Crossterm, small reusable widgets, and immutable view models. Add `crates/tui`; keep application composition in `crates/app`. Reuse domain types.

## Data and execution

Publish immutable `DashboardSnapshot` values with system, safety, camera, detection, tracking, prediction, aiming, latency, and session metrics. Include frame identity, source mode, timestamps, and data availability. Widgets must not access mutable pipeline or hardware state.

Keep rendering outside the processing critical path. Target 10 to 20 updates per second. Use bounded nonblocking publication and discard stale snapshots. A full channel, stopped reader, slow renderer, or disconnected TUI must not delay processing. Send controls through a separate bounded channel; the domain owner validates them.

Bound event history, trajectories, tracks, and chart buffers. Calculate statistics outside rendering. Prepare a downsampled monochrome viewport outside drawing; keep its renderer replaceable. Avoid large snapshot clones and complete frame copies. Measure publication, aggregation, and rendering costs.

## Safety presentation

Show safety on every screen and at every usable terminal size. Lockout or fault must override the normal visual hierarchy. Show the reason and output status. Cover human, dog, cat, unavailable detector, detector error, timeout, stale safety, stale camera, insufficient confidence, and invalid state.

Safety authority remains in the safety subsystem and output interlock. The TUI must never provide a bypass, disable command, forced aim, or output override. A stale or missing dashboard snapshot cannot prove current safety: show unavailable or stale status and do not claim current output permission. Pause must not freeze the current safety header. During replay, distinguish recorded safety evidence from current live safety authority.

## Screens

| Key | Screen | Required information |
| --- | --- | --- |
| 1 | Live, default | Observed position, recent trajectory, predicted position, requested aim, region boundary, target details, stage latency, latency history, session counters, recent events |
| 2 | Performance | Latency, prediction error, continuity, dropped frames; 1-minute, 5-minute, 15-minute, 1-hour and session windows; current, previous and best comparable runs |
| 3 | Tracks | Active and recent tracks; ID, lifecycle, age, confidence, speed, lost count and error; selected measured, estimated and predicted trajectories; loss reasons |
| 4 | Safety | Global state; human, dog and cat confidence; inference health and age; camera and result freshness; watchdog; output interlock; bounded lockout history |
| 5 | Calibration | Point positions and accepted/rejected/failed status; residuals, RMS and maximum error; version and timestamp; selected point details |

The persistent header shows LIVE or REPLAY, safety, camera health, FPS, selected track, aiming mode, and uptime. Target details include ID, lifecycle, confidence, age, velocity, prediction horizon, available prediction error, continuity, losses, and reacquisitions.

Show capture, safety, detection, tracking, prediction, aiming, and end-to-end latency. Expose P50, P95, P99 and maximum, with units and sample window. Do not make mean latency the primary statistic. Measure end-to-end latency directly; concurrent stage durations need not add to that value.

Session counters include acquired/lost targets, reacquisitions, lockouts, processed and dropped frames, and ground-truth false positives. Missing ground truth means unavailable, not zero. Show detection, prediction and virtual aiming error and false negatives when ground truth permits.

## History and comparisons

Persist versioned `RunMetrics` after a completed run, outside the processing path. Include timestamp, optional Git commit, dataset/fixture and configuration identities, latency distributions, and detection, tracking, prediction and safety statistics. Git must remain optional.

Each metric defines its units and preferred direction. Show underlying values and regressions, not an aggregate score. Compare equivalent datasets, configurations, hardware and metric definitions. Define best per metric within that cohort. Treat a zero baseline explicitly; use percentage points for continuity changes. Bound long-session charts through aggregation.

## Controls and terminal behavior

Use explicit enums for input, telemetry, replay controls and application commands. Keys 1 to 5 select screens; Tab changes focus; arrows select targets when that focus applies; `?` opens compact help; `q` exits the dashboard. Show context-specific controls in the footer.

Replay uses the same screens and pipeline. Show fixture, frame/total, speed and ground-truth status. Space pauses/resumes; Right steps when paused; Left rewinds when supported; Shift plus arrows takes a larger step. Route conflicting arrows by focus. Seek must restore deterministic state, for example by replaying from a checkpoint, rather than showing an old image with current tracks.

Expose `r`, `R`, and `s` calibration retry/restart/save only when supported and safe. Recording is context-specific and must not conflict with calibration controls. Unsupported actions show an unavailable state. Domain handlers recheck safety when executing commands.

Use green for healthy, yellow for uncertain, red for lockout/fault, cyan for targets/prediction/aiming, and muted text for history. Always use text or symbols with color. Provide a consistent observed/trajectory/predicted/aim legend and a basic-terminal fallback.

Implement LARGE, MEDIUM and MINIMAL layouts. Small terminals preserve safety, system state, selected target, critical latency, and essential controls, in that order. Handle zero/tiny dimensions and long text without panic. Restore raw mode, cursor and alternate screen on exit and recoverable errors. Keep structured logs away from the drawing terminal.

## Validation

Use Ratatui TestBackend and semantic assertions for safety. Test each layout, SAFE and lockout, missing/stale telemetry, no/one/multiple targets, replay, comparisons, calibration failure, and text overflow. Test stalled/disconnected consumers, bounded memory, command rejection, deterministic seek and terminal cleanup.

Benchmark with the same replay input with and without TUI, including a stalled renderer. Report P50/P95/P99/max publication and draw duration, pipeline latency, dropped frames, memory and CPU. Measure Raspberry Pi CPU when hardware is available. Do not claim a throughput target or acceptable overhead without measurement and an explicit budget.
