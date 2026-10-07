# NanoDet safety integration

## Decision

Use NanoDet-Plus as the selected model family for the safety detector. The user reports successful detection of partial humans and partial dogs in the [ncnn web demo](https://nihui.github.io/ncnn-webassembly-nanodet/). Use its implementation as the integration reference. Do not substitute a different checkpoint without recording and testing the change.

## Reference configuration

Inspected upstream revision: `ac68cf74d4d81f70160bb9e9020c8be1fd78502a` in [nihui/ncnn-webassembly-nanodet](https://github.com/nihui/ncnn-webassembly-nanodet/tree/ac68cf74d4d81f70160bb9e9020c8be1fd78502a). The current source is a reference; it does not prove which revision was deployed when the user tested the demo.

- Runtime: ncnn through WebAssembly in the demo. Use native ncnn as the proposed Pi integration route.
- Model basename passed to the loader: `coco.torchscript.ncnn`.
- Input: RGBA converted to BGR. Resize with preserved aspect ratio, then pad. Target size: 416 pixels.
- BGR means: `[103.53, 116.28, 123.675]`. Normalization divisors: `[57.375, 57.12, 58.395]`.
- Input tensor: `in0`. Output tensors: `231`, `228`, `225`, `222`, for strides 8, 16, 32, 64.
- Zero-based hazard class indexes: person 0, cat 15, dog 16.
- Demo score threshold: 0.4. Demo NMS threshold: 0.5. These are reference settings, not validated safety acceptance values.

Verify the model assets and preprocessing against the pinned source before implementation. Record asset checksums and confirm weight licensing. Test replicated grayscale channels with the same BGR normalization; do not assume that color-demo results transfer to the OV9281.

Keep insect detection independent. Integrate the image backend through `safety::SafetyDetector`. Do not read synthetic ground truth in the image backend.

## Current implementation

The working tree has a safety detector interface, a fixture detector, an unavailable detector, and a safety authority. The authority checks frame identity, timestamps, confidence, evidence scope, and faults before it executes a command. The fixture detector reads labels; it does not recognize objects in images.

The NanoDet prototype now has a Rust adapter and a persistent Python worker that calls native ncnn. Rust owns the verdict, a bounded pipe request channel, frame identity checks, and the request deadline. A timeout or protocol fault stops the worker; late evidence cannot be reused. Hazard scores below the configured threshold produce uncertain evidence, never clearance.

The worker uses the reference preprocessing and checks all three protected class logits before class selection or box suppression. This is deliberately more conservative than the demo. It returns presence scores only; it does not decode or display boxes. The model remains loaded between frames. The Rust adapter currently copies one monochrome frame for pipe transfer, and the worker allocates preprocessing tensors. Measure before optimization.

The dashboard app now connects this backend through explicit replay configuration. See [image replay safety](replay-image-safety.md) for launch commands and limits. The Python bridge is a prototype dependency, not a decision to use Python in the final Pi deployment. A small native bridge remains the proposed final integration route. These components do not establish live safety readiness.

## Run the offline evaluation

The local `.venv-safety` environment now contains native ncnn and NumPy. The pinned model assets are in `models/nanodet-reference`. Model files and the local runtime are excluded from Git. See `tools/safety/requirements-test.txt` for validated direct dependencies and `requirements-test.lock.txt` for the complete local dependency set. Weight licensing remains open before redistribution.

Use an image-sequence JSON manifest supported by `camera::replay::ImageSequence`, with binary PGM frames. Run from the repository root:

```powershell
cargo run -p safety --example nanodet_replay --release -- .venv-safety/Scripts/python.exe tools/safety/nanodet_worker.py models/nanodet-reference/coco.torchscript.ncnn.param models/nanodet-reference/coco.torchscript.ncnn.bin PATH_TO_SEQUENCE_JSON
```

This command logs per-frame hazard scores, verdicts, cold startup time, warm P50/P95/P99/maximum latency, and missed labelled hazards. It has no aiming device. A detector fault stops evaluation with an error. The example uses a 10-second offline deadline and a 0.4 score threshold; these values must not become live safety limits without validation. Timings include preprocessing, transfer, and inference. This example does not measure an independent live watchdog or entry-to-lockout latency.

## Prototype validation

Rust unit tests check hazard mapping, uncertain empty results, invalid numeric scores, and identity rejection. Optional process tests use an explicitly labelled test double to check persistent requests, timeout, crash, mismatched responses, and zero output through the safety gate. Run these process tests with an existing Python interpreter:

```powershell
$env:FLY_TRACKER_TEST_PYTHON = 'C:\Python312\python.exe'
cargo test -p safety --test nanodet_process -- --ignored
```

On the development Windows host, the protocol benchmark transferred 101 monochrome 640 x 480 frames through the test double. Cold startup: 35,564 microseconds. For 100 warm requests: P50 167, P95 212, P99 243, maximum 296 microseconds. These results measure host protocol overhead only, not NanoDet inference or Pi performance.

## Real inference validation

The local environment was created with user authorization. Python 3.12 and ncnn 1.0.20260526 successfully ran the pinned model through `NanoDetDetector::evaluate` in Rust. Persistent calls succeeded for 640 x 480, 480 x 640, 416 x 416, and 320 x 192 frames. A missing model caused an error verdict. Protocol tests also passed for worker timeout, crash, mismatched response, and zero issued output.

Downloaded smoke images: [Ultralytics bus image](https://github.com/ultralytics/ultralytics/blob/main/ultralytics/assets/bus.jpg) and [PyTorch Hub dog image](https://github.com/pytorch/hub/blob/master/images/dog.jpg). `prepare_test_images.py` converts them to monochrome, preserves aspect ratio, and pads to 640 x 480. They are local smoke fixtures, not a representative safety dataset. Both caused a protected-class hazard verdict and zero permitted commands. The human score was 0.77385. The dog image was classified as cat: cat score 0.61966, dog score 0.37290. The first exact-class assertion failed and exposed this limit. The final safety test checks protected-class presence and output suppression; the replay report records class mismatches separately from missed protected hazards. Do not treat this as evidence of accurate dog classification.

For 101 blank monochrome 640 x 480 frames through the real worker on the Windows development host, cold startup was 160.291 ms. For 100 warm requests: P50 14.395 ms, P95 17.208 ms, P99 18.440 ms, maximum 19.122 ms. Timings include pipe transfer, preprocessing, and native inference. They do not establish Pi performance or representative-scene worst-case latency.

Run the real inference tests, fixture preparation, and checksum checks again:

```powershell
& ./tools/safety/Test-NanoDet.ps1
```

To recreate the local environment, run `python -m venv .venv-safety`, then `.venv-safety/Scripts/python.exe -m pip install -r tools/safety/requirements-test.lock.txt`. Download the two model files from the pinned upstream `assets` directory into `models/nanodet-reference`. Download the two linked smoke images into `.test-dist/nanodet` as `bus.jpg` and `dog.jpg`. The test script checks both model and image checksums before it runs inference.

Model SHA256 checksums:

- `coco.torchscript.ncnn.param`: `1c0ddf1e8009a6ff55032deda21b97e221ebdfe68d820faa68999790d5e972f9`.
- `coco.torchscript.ncnn.bin`: `1942dede585c2163c888e34295fffffdf73841ca8e79b2a548042393510fb9ae`.

The Rust inference call is tested locally and ready for replay integration. Production safety validation, edge hands/arms, partial animals, real cat photographs, camera recordings, live watchdog integration, Pi memory, and Pi latency remain open. The detector never grants clearance. Physical output remains disabled.

## Next delivery

1. Record the tested variant, weights path, file checksum, license, runtime, input dimensions, normalization, resize method, class mapping, and output decoding rules.
2. Add an image backend behind `SafetyDetector`. Validate frame data before inference. Preserve frame identity and acquisition time in the evidence. Distinguish live input from replay input.
3. Map person, dog, and cat detections to hazard verdicts. Model load failures, inference failures, invalid output, and timeouts must cause lockout.
4. Return uncertain evidence when the detector cannot establish clearance. An empty list of detections must not automatically become `Clear(1.0)`. Define and validate the clearance policy separately from detection scores.
5. Test full and partial humans, edge hands, edge arms, full and partial dogs, full and partial cats, and each animal or human together with a fly. Use representative monochrome images from the intended camera setup. Keep control-logic tests separate from image-recognition tests.
6. Measure inference and entry-to-lockout latency on the Raspberry Pi 4. Report P50, P95, P99, maximum, memory use, missed hazards, and false lockouts. Set freshness limits from the required response time and measured results.
7. Connect the backend and an independent watchdog in one pipeline owner. Recheck safety at command execution. Keep physical output disabled until the complete safety gate passes.

## Acceptance and limits

Run workspace formatting, strict Clippy, and tests for code changes. Every safety-positive control fixture must produce lockout and zero aiming commands. Verify expiry when frames stop and suppression after detector failure.

The user's trial supports model selection. It does not yet establish coverage for cats, edge hands or arms, monochrome input, or the installed field of view. No live clearance or physical output is enabled by this decision.
