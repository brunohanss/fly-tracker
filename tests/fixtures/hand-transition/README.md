# Edge-hand reaction regression

The supplied PNG files are copied unchanged to `clear.png` and `hand.png`. The second image contains a hand and part of an arm entering from the lower-right edge. Both images retain the fixed fly marks. No image generation or pixel edits were used. The PGM files are grayscale replay conversions.

Five window fly marks are annotated at approximately (441,255), (559,260), (761,311), (777,332), and (546,379). Other fly marks remain visible on the right wall but are outside this fixture's insect region. The selected region is x=420, y=240, width=370, height=150. The initial larger region included a small background candidate near (799,392); narrowing the region excluded it. Safety inference always receives the full frame, including the hand outside this region.

| Manifest | Input order | Source interval |
| --- | --- | --- |
| hand-entry.json | Five clear images, then five hand images | 200 ms |
| hand-exit.json | Five hand images, then five clear images | 200 ms |
| hand-cycle.json | Five clear, five hand, five clear | 200 ms |
| hand-reactivity.json | Five clear, then hand/clear/hand/clear/hand | 10 ms |

The repeated phases use identical pixels. Manifests reuse two PGM source files. Ten numbered PNG copies are available locally in `target/hand-transition-preview`. The short source interval tests frame order and evidence isolation; it does not claim 100 FPS live inference.

## Run the replay

```powershell
cargo run --release -p app -- --config tests/fixtures/hand-transition/config.json --sequence tests/fixtures/hand-transition/hand-cycle.json
```

The configuration explicitly uses fixture labels. Partial human visibility is represented by `Human` hazard metadata. The entry sequence issues three virtual requests after confirmation, then suppresses all five hand-positive requests. The exit sequence resumes on its first fresh clear frame. The single-frame reentry test checks each new hand frame, including the first frame after an accepted clear request. Tracking continues during lockout. Physical output remains disabled.

## Automated validation

`cargo test --workspace` includes four edge-hand control tests: entry, exit, the full cycle, and single-frame reentry. Shared checks verify the five target coordinates and track IDs, confirmed tracking during lockout, zero commands on every hand-positive frame, current-frame predictions, and deterministic seek.

Run real image recognition with the installed [runtime and pinned weights](../../../docs/nanodet-safety-integration.md):

```powershell
cargo test -p app --features nanodet-model-tests --test hand_transitions
```

This test requires successful inference. Every hand-positive frame must produce a `Human` hazard verdict; uncertainty alone cannot pass that assertion. Clear-image results remain uncertain because image clearance is still unvalidated. Therefore accepted aiming transitions use fixture evidence; real inference issues zero commands throughout.

## Measured reaction and limitations

NanoDet recognized the hand as human on the first hand frame and all five repeated hand frames in the measured entry replay. Human presence score: 0.437601. Configured hazard threshold: 0.4. No threshold change was made for this fixture. This is a narrow margin and one generated pose; it does not establish hand coverage across sizes, positions, occlusions, or lighting.

The first hand-frame inference took 22.602 ms on the Windows host, including IPC and preprocessing. Across nine warm requests, inference P50/P95/P99/maximum was 22.018/23.140/23.140/23.140 ms. Cold startup was 177.926 ms. Cold loading exceeds the normal 50 ms freshness limit and must remain locked out. These timings measure inference response to a supplied frame, not physical hand-entry-to-output-disable latency. Live camera cadence, scheduling, independent watchdog behavior, and Raspberry Pi timings remain unvalidated.

The final 15-frame release fixture cycle retained five tracks and issued eight virtual requests, with all five hand-positive requests suppressed. Complete frame processing P50/P95/P99/maximum was 13.708/17.690/17.690/17.690 ms. Frame-to-command P50/P95/P99/maximum was 13.467/13.773/13.773/13.773 ms across eight accepted requests. Formatting, strict Clippy, workspace tests, and all five feature-enabled hand tests passed.

Source PNG SHA256 checksums:

- `clear.png`: `4c8d08a2443005a5ecd01190577e68512552714b1d98dfe495784d42971ed360`.
- `hand.png`: `7cd315ae7fddf1180136ff17a8140daf108712d447be598c7f922d4d99bc6c32`.

Rebuild into a new directory with:

```powershell
.\tools\New-HandTransitionFixture.ps1 -ClearImage tests/fixtures/hand-transition/clear.png -HandImage tests/fixtures/hand-transition/hand.png -OutputDirectory target/new-hand-transition
```
