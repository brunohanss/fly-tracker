---
name: fly-tracker-development
description: Plan, implement, or review the Fly Tracker Rust workspace, including fail-closed safety, deterministic replay, tracking, telemetry, and the Ratatui observability dashboard.
---

# Fly Tracker development

Read the repository `AGENTS.md` and `README.md` completely. Inspect the affected crates and tests before changing code. Treat `AGENTS.md` as the engineering contract. Use PowerShell commands and ASD-STE100-style instructions. Use Mermaid to explain complex flows.

Select one independently testable milestone. State the smallest coherent change, its prerequisites, acceptance checks, and known limits. Do not treat starter functions or mock data as implemented subsystems.

Keep safety independent from insect detection. SafetyLockout overrides tracking and all aiming output, including virtual commands. Missing, failed, stale, or uncertain safety evidence must fail closed. Do not implement a hazardous emitter.

Keep camera, detection, tracking, prediction, aiming, hardware, and telemetry boundaries explicit. Reuse typed coordinates and monotonic timestamps. Keep queues and buffers bounded. Do not add Tokio without a measured requirement.

For dashboard work, read [dashboard requirements](references/dashboard.md). For the initial delivery sequence and repository integration points, read [the implementation plan](../../../docs/ratatui-implementation-plan.md). Recheck the repository state before using that plan.

Before marking an implementation milestone complete, run:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Run relevant benchmarks. Report measured results and limits. Documentation-only changes require link and skill validation; do not claim that they validate runtime behavior.
