# Image replay safety and sequential virtual aiming

The replay app can run NanoDet-Plus through native ncnn on each complete grayscale frame. Physical output stays disabled. A presence score is not a clearance confidence. NanoDet still returns `Uncertain` when no protected class meets the hazard threshold. It cannot yet authorize aiming from images alone.

## Run the stationary virtual aim demo

The original room sequence has five frames. Tracks confirm on frame three. Only three frames remain for aiming. Create a longer labelled fixture to show all five targets:

```powershell
.\tools\New-VirtualAimDemo.ps1 -Sequence target/repeated-room-images/image-1/sequence.json -OutputDirectory target/my-virtual-room-demo
cargo run --release -p app -- --config config/stationary-room-virtual.json --sequence target/my-virtual-room-demo/sequence.json --fixture-safety
```

The output directory must be new. This command explicitly uses recorded fixture labels. It does not prove that an image is clear. The tool copies one existing frame and retains its target and hazard annotations. It writes 1,000 entries at 10 ms source intervals.

The scheduler selects confirmed tracks in increasing ID order, then starts again. It sends one virtual request per processed frame. `processing.virtual_aim_dwell_us` holds the selected target for a source-time interval. The demo uses 200,000 us. Each frame uses the current prediction. No coordinate queue exists. A removed or lost target is excluded. The scheduler advances only after an accepted request. A lockout cannot advance to an unprotected command. This is an aim display, not a pulse or firing sequence.

The Live view shows `+` for an issued virtual request and `x` for a suppressed request. It also shows the requested target ID and normalized XY coordinates. These coordinates describe the virtual calibration plane. They are not physical galvo instructions.

## Select real image inference

Use the existing local Python environment and pinned model files described in [NanoDet setup](nanodet-safety-integration.md). Paths are relative to the process working directory. Run from the repository root:

```powershell
cargo run --release -p app -- --config config/stationary-room-nanodet.json --sequence target/repeated-room-images/image-1/sequence.json
```

The `safety_backend` object selects `unavailable`, `fixture`, or `nanodet`. The default is `unavailable`. `fixture` is explicit label-based test mode. The CLI rejects `--fixture-safety` when a backend is already configured. Synthetic input retains its existing labelled simulation default.

NanoDet settings require Python, worker, parameter and weight paths; a hazard threshold in (0,1]; a timeout from 1 to 60,000,000 us; and 1 to 4 worker threads. `max_safety_age_us` and `max_frame_age_us` are separate freshness limits. The supplied configuration uses a 1-second inference timeout and 50-ms freshness limits. A result can meet its inference timeout but fail freshness. Cold model loading can exceed freshness. These are host replay settings, not validated Pi or live-camera limits.

One pipeline owner calls inference synchronously. The existing worker channel holds at most one request. Each response must retain the exact frame ID and source timestamp. Image evidence uses `ReplayImage` scope. It never becomes live evidence. Model initialization failure leaves `DetectorUnavailable` visible. Later errors and timeouts stop the worker and suppress requests. Real processing time is checked against both freshness limits before commands. Prior clear evidence cannot authorize a later frame.

The Live and Safety views show backend health, presence scores, elapsed inference time, evidence age, and the suppression reason. Replay clearance remains distinct from live authority.

## Validation on the Windows development host

Control tests cover clear evidence, all three hazard classes, uncertainty, low confidence, unavailable/error/timeout/stale evidence, identity and scope errors, and slow inference. They also check stationary tracking during lockout, current prediction coordinates, cycle order, dwell, and deterministic seek. These use a controlled backend, not claimed image recognition.

The existing real-model tests pass on the available human and dog smoke photographs. They also test real initialization failure and repeated inference. Edge hands, arms, partial animals, and real cat photographs are still missing. Generated room images are smoke fixtures only. The dog photograph can be classified as cat; both classes must suppress output.

For 1,000 repeated 1448 x 1086 room frames in release mode:

| Metric | P50 ms | P95 ms | P99 ms | Maximum ms |
| --- | ---: | ---: | ---: | ---: |
| NanoDet safety stage, including startup | 21.047 | 24.866 | 31.428 | 167.127 |
| Complete frame processing, NanoDet | 34.184 | 38.662 | 46.476 | 184.186 |
| Frame to accepted virtual command, fixture labels | 12.817 | 14.232 | 17.104 | 29.839 |

NanoDet issued zero commands and suppressed 998 requests. Thus its frame-to-command distribution is unavailable. The labelled demo issued 998 requests after confirmation. These two runs use different safety backends. Do not compare them as equivalent detector accuracy tests.

Observed app working-set maximum was 39.19 MiB, sampled approximately every 500 ms. The worker memory query was denied during this run; worker and total memory remain unmeasured. Host timings include image transfer and preprocessing. They do not establish Raspberry Pi latency, memory, watchdog response, or production safety readiness.

## Remaining acceptance gap

A validated image clearance policy and an annotated protected-class dataset are still required. Do not convert low human/dog/cat presence scores to `Clear(1.0)`. The image backend is connected, but real-image clearance acceptance is incomplete. Use explicit labelled fixtures to inspect virtual aim scheduling while that work remains open.

The installed weight file is 2,397,184 bytes (2.29 MiB), with a 17,560-byte parameter file. File size does not give peak runtime memory. A 1 GB Pi is not automatically excluded: [ncnn supports Raspberry Pi and provides memory allocators](https://github.com/Tencent/ncnn). The Python/NumPy bridge is a development prototype. Measure the exact model and complete pipeline on the 1 GB Pi before choosing the deployment runtime or accepting latency limits.
