# Stationary-target test

The AI-motion sequence is retired. Each of the six supplied room images is tested separately as five byte-identical grayscale frames. No motion, blank reference image, or target removal is generated. The original PNG files remain unchanged. Timestamps use an assumed 10 ms interval.

## Detection change

The earlier background-difference detector found zero targets in all six tests. It initialized its background from the first frame, which already contained the flies. This was a detection failure, not a successful stationary-target test.

The detector now combines temporal change with local spatial contrast. A dark pixel that is sufficiently darker than its 17 by 17 neighborhood enters the foreground mask. The existing neighborhood cleanup, connected components, and geometry filters then produce detections. This path works on the first frame and on unchanged later frames. It uses reusable summed-area storage and requires no target coordinates as input.

`processing.stationary_detection` defaults to `true`. Set it to `false` only when a motion-only comparison is required. `processing.detection_region` can select the insect observation area. It does not crop the safety detector input or weaken safety checks.

The [room configuration](../config/stationary-room.json) selects the exposed wall: x=470, y=35, width=750, height=610 pixels. This excludes furniture, plants, and the floor. This region is specific to these images. It must not be copied to another camera view without checking its bounds and coverage.

## Verified result

All six images produced five detections in every repeated frame: 30 tested frames and 150 detections. Each sequence confirmed five persistent tracks. No target was missed and no candidate was unmatched within the selected wall region, using a 6-pixel matching tolerance against [manual approximate annotations](../config/stationary-room-targets.json). These annotations describe generated target marks. They do not establish real-insect recognition. They are used only for scoring, not for image detection.

No aiming commands were issued in the image tests because live image safety evidence is unavailable. Separate synthetic regression tests confirm stationary targets present from the first frame, zero estimated velocity, virtual aiming after confirmation with clear fixture evidence, and zero commands when a human fixture forces lockout.

Run a stationary room fixture:

```powershell
cargo run --release -p app -- --config config/stationary-room.json --sequence target/repeated-room-images/image-1/sequence.json
```

The fixture paths are local build artifacts. Image numbers 1 to 6 correspond to PNG filename suffixes 1 to 6. Press `m` to inspect the foreground mask. The replay stops after five frames. Safety stays locked out.

To see accepted virtual aims at all five targets, use [the extended labelled virtual demo](replay-image-safety.md). It uses explicit fixture safety and a 200 ms target dwell. The same guide gives the real NanoDet replay configuration and its remaining clearance limits.

Use [the repeated-image tool](../tools/Test-RepeatedImages.ps1) to rebuild tests in a new directory:

```powershell
cargo build --release -p app
# Specify the six PNG paths in suffix order 1 to 6.
.\tools\Test-RepeatedImages.ps1 -Images $images -OutputDirectory target/new-still-tests -Config config/stationary-room.json -Annotations config/stationary-room-targets.json
```

For an independent coordinate check on an annotated fixture:

```powershell
cargo run --release -p app --example inspect_stationary -- target/repeated-room-images/image-1/sequence.json config/stationary-room.json target/new-inspection.json
```

The output lists per-frame detections, missed annotated targets, and unmatched candidates. The checker requires exactly five identical frames. It matches coordinates within 6 pixels. Output files must be new paths.

## Limits

Spatial contrast identifies small dark objects, not insect species. Stains, dirt, and small texture features can also produce candidates. Bright, blurred, or low-contrast insects can be missed. The wall region is necessary for this cluttered room fixture: the initial larger region included five extra candidates near furniture and plants. Real wall recordings, stationary real insects, and distractor annotations are still required before claiming real-world accuracy.

The summed-area buffer adds about 12 MiB at 1448 by 1086 pixels, or about 0.59 MiB at 320 by 240. It is allocated during initialization and reused. Measure throughput on the selected hardware. Historical performance measurements in the implementation status document predate this detection path.

The updated 2,000-frame release benchmark at 320 by 240 pixels measured headless pipeline P50/P95/P99/max of 511.5/686.3/948.3/1146.5 microseconds. Normal dashboard mode measured 537.5/839.1/1027.7/2603.5 microseconds. These distributions cover the final 1,024 samples. Total wall time was 1.060 seconds headless and 1.241 seconds with the dashboard. All four modes, including stalled and disconnected readers, dropped zero replay frames. The new path costs more than motion-only detection. These host results do not establish Raspberry Pi performance or live overload behavior.

Physical output remains disabled. A stationary detection never overrides unavailable, stale, failed, or hazard-positive safety evidence.
