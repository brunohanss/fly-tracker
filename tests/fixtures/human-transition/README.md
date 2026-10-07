# Supplied room images with human entry and exit

The supplied PNG files are copied unchanged to `clear.png` and `human.png`. All nine fly marks remain visible in both images. No image generation or pixel edits were used. The PGM files are grayscale replay conversions.

| Manifest | Frames 1–5 | Frames 6–10 | Frames 11–15 |
| --- | --- | --- | --- |
| human-entry.json | Image 1: fixed flies, no human | Image 2: same flies plus child | — |
| human-exit.json | Image 2: fixed flies plus child | Image 1: same flies, no human | — |
| human-cycle.json | Image 1 | Image 2 | Image 1 |

Each five-frame phase repeats byte-identical pixels. Source timestamps advance by 200 ms per frame. Manifests reuse the two PGM files to avoid storing duplicate pixels. Ten numbered PNG copies are also available locally in `target/human-transition-preview`.

The nine approximate target centers are (514,164), (1027,170), (811,222), (1204,227), (590,251), (725,346), (1001,401), (504,424), and (1251,441). These are manual test annotations, not detector inputs. The insect region is x=470, y=100, width=850, height=360. It excludes furniture, posters, and plants. Safety inference receives the complete frame, including the child outside this region.

## Run the replay

```powershell
cargo run --release -p app -- --config tests/fixtures/human-transition/config.json --sequence tests/fixtures/human-transition/human-entry.json
```

Use `human-exit.json` for lockout → virtual aiming, or `human-cycle.json` for aiming → lockout → aiming. The configuration explicitly selects fixture-label safety. Physical output remains disabled.

## Automated checks

`cargo test --workspace` includes four new tests: entry, exit, the full cycle, and byte-identical repeats. Shared transition checks verify all nine target positions within six pixels, persistent track IDs, confirmed tracking during lockout, zero requests on human frames, immediate resumption on fresh clear fixture evidence, and deterministic seek. The dog regressions use the same checks.

The entry replay issues three virtual requests after track confirmation, then suppresses five human-positive requests. The exit replay issues five requests after the human leaves. The cycle issues eight requests and suppresses five. Every human-positive frame permits zero commands.

Run the optional real-model test with the installed [runtime and pinned weights](../../../docs/nanodet-safety-integration.md):

```powershell
cargo test -p app --features nanodet-model-tests --test human_transitions
```

On Linux, set `FLY_TRACKER_TEST_PYTHON` to the safety environment interpreter. NanoDet detected the child as human in every human-positive frame across all three sequences on the development Windows host. The model test also checks that inference is ready and no commands are issued. Dog-free/human-free scores remain uncertain: real image clearance is still unvalidated. Accepted aiming transitions are checked with explicit fixture labels.

## Measurements and limits

The 15-frame release fixture cycle processed 135 detections and retained nine tracks. It issued eight virtual requests, suppressed five human-positive requests, and issued zero unsafe commands. Complete frame processing P50/P95/P99/maximum was 13.342/17.103/17.103/17.103 ms. Frame-to-command P50/P95/P99/maximum was 13.151/13.622/13.622/13.622 ms from eight accepted requests. These small host samples do not establish Pi performance or representative human detection accuracy.

These supplied images are generated room smoke fixtures. They do not validate partial humans, edge hands, or arms. Physical output remains disabled.

Source PNG SHA256 checksums, verified unchanged after copying:

- `clear.png`: `5658f67e7ac0b762843cb2271966c8cb9de59a4007dffebf1d7e44f171dcb831`.
- `human.png`: `3ad6fb1ee3c7e0fe2d4d63930b2d843219dd89df07f9875ebcd7deaccdaae4d5`.

To rebuild into a new directory:

```powershell
.\tools\New-HumanTransitionFixture.ps1 -ClearImage tests/fixtures/human-transition/clear.png -HumanImage tests/fixtures/human-transition/human.png -OutputDirectory target/new-human-transition
```
