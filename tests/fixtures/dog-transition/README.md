# Fixed flies with dog entry and exit

All frames contain the same five fixed fly marks at approximately (503,126), (1059,144), (1014,307), (602,460), and (1100,583). Coordinates use the 1448 x 1086 source image. The dog stays below the insect detection region. Safety inference receives the complete image.

The two source images repeat within each phase. The manifests represent ten or fifteen pictures without storing duplicate pixel files. `clear.png` is the existing generated room fixture converted from PGM. `dog.png` was produced with the built-in image generation tool, using `clear.png` as the edit target. The selected prompt is recorded in [generation-prompt.txt](generation-prompt.txt). No pixel edits were made after generation; `000000.pgm` and `000001.pgm` are grayscale format conversions for replay.

| Manifest | Frames 1–5 | Frames 6–10 | Frames 11–15 |
| --- | --- | --- | --- |
| dog-entry.json | Five flies, no dog | Same flies plus dog | — |
| dog-exit.json | Five flies plus dog | Same flies, no dog | — |
| dog-cycle.json | Five flies, no dog | Same flies plus dog | Same flies, no dog |

Each source interval is 200 ms. `config.json` explicitly selects fixture-label safety and the wall detection region. Track confirmation requires three observations. Virtual targeting cycles once per frame. Physical output is disabled.

Run the ten-frame entry sequence:

```powershell
cargo run --release -p app -- --config tests/fixtures/dog-transition/config.json --sequence tests/fixtures/dog-transition/dog-entry.json
```

Use `dog-exit.json` to inspect lockout → aiming. Use `dog-cycle.json` to inspect aiming → lockout → aiming. Frames 1 and 2 establish candidate tracks. In the entry sequence, frames 3–5 issue three virtual requests; frames 6–10 issue zero. In the exit sequence, all five dog-free frames issue requests because tracking continued during lockout. Each of the five tracks receives a request.

## Automated checks

`cargo test --workspace` runs the three tests in `crates/app/tests/dog_transitions.rs`. It checks every frame, retained track IDs, all five target coordinates, zero commands on dog frames, the first valid clear frame after exit, and deterministic seek. These tests use fixture labels to test the safety authority and command interlock. They do not claim image clearance accuracy.

To run real NanoDet inference on these same pictures, install the [documented runtime and pinned weights](../../../docs/nanodet-safety-integration.md), then run:

```powershell
cargo test -p app --features nanodet-model-tests --test dog_transitions
```

On Linux, set `FLY_TRACKER_TEST_PYTHON` to the interpreter in the safety environment. On Windows, the test defaults to `.venv-safety/Scripts/python.exe`. The real-model test fails when initialization or inference fails. It has no label fallback. It checks all three sequences: every dog frame must produce `Dog` lockout, and every dog-free frame must remain uncertain. No real-model frame can issue a virtual command because image clearance is not yet validated.

## Verified result and limits

Workspace formatting, strict Clippy with all features, and workspace tests passed. The real-model transition test also passed on the Windows host with the installed ncnn runtime and pinned weights. NanoDet recognized the generated dog in all 15 dog-positive frames across the three sequences. This single generated dog is a smoke fixture, not evidence of representative animal detection accuracy.

The 15-frame release fixture replay issued 8 virtual requests, suppressed 5 dog-positive requests, and issued zero unsafe commands. Complete frame processing P50/P95/P99/maximum was 12.872/22.331/22.331/22.331 ms. Frame-to-command P50/P95/P99/maximum was 12.795/13.412/13.412/13.412 ms, from 8 accepted requests. These small host samples do not establish Pi performance.

Rebuild into a new directory with:

```powershell
.\tools\New-DogTransitionFixture.ps1 -ClearImage tests/fixtures/dog-transition/clear.png -DogImage tests/fixtures/dog-transition/dog.png -OutputDirectory target/new-dog-transition
```

The numbered preview exports for the requested ten pictures are in the local `target/dog-transition-preview` directory. Build output is not required by Cargo tests; the two PGM source files and manifests in this directory are sufficient.
