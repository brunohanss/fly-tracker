# Six-image room replay

This moving-image test is retired at the user's request. Use [the repeated-image test](repeated-image-test.md) instead. Do not use AI-generated motion to measure trajectory accuracy.

Test date: 7 October 2026. Input: the six supplied PNG images, in suffix order `-1` to `-6`. Resolution: 1448 by 1086. Original files remain unchanged.

The importer converted the files to monochrome PGM and wrote [the replay manifest](../target/room-images/sequence.json). It assigned a one-second interval. This interval is a test assumption; filename times do not establish camera acquisition times. No target annotations or safety evidence were added.

Run the dashboard:

```powershell
cargo run --release -p app -- --sequence target/room-images/sequence.json
```

Press Space to pause, then Left or Right to inspect frames. Press `m` to inspect the foreground mask. The six-frame replay stops after completion. Safety remains locked out because the detector is unavailable in this runner.

## Result

All six frames loaded and processed. No frames were dropped. No aiming command was issued. The [report](../target/room-images-run.json) records 640 detections, 32 confirmed tracks, and five frames at the detection capacity limit. The first frame initialized the background; each later frame reached the 128-detection limit. Confirmed tracks alone do not prove that insects were tracked correctly.

Frame-processing P50/P95/P99/max was 9191.2/9675.4/9675.4/9675.4 microseconds on the Windows host in release mode. There are only six samples. These measurements are not a sustained throughput result. Accuracy and identity errors are unavailable because target ground truth is absent.

The images have differences in wall texture and scene objects as well as target positions. The background-difference detector treats these changes as foreground. These generated images can test file loading, fault handling, and detector behavior under scene changes. They do not establish real insect accuracy or prediction accuracy.

For a controlled sequence, use the same background pixels in each frame and change only the insect positions. Include an empty background frame for warm-up and record target coordinates and explicit timestamps. Keep such a fixture labelled as synthetic. For real-image validation, record a fixed-camera video and annotate target positions. Do not infer safety clearance from the absence of visible people or animals in an unlabelled image.

## Import other images

Use the [PowerShell importer](../tools/Import-ImageSequence.ps1). Specify the input paths in temporal order. The destination must be a new directory. Images must have the same dimensions. The importer preserves resolution and uses an integer grayscale conversion.

```powershell
$images = @('C:\recordings\frame1.png', 'C:\recordings\frame2.png')
.\tools\Import-ImageSequence.ps1 -Images $images -OutputDirectory target/my-images -PeriodUs 10000
cargo run -p app -- --sequence target/my-images/sequence.json
```

Set `PeriodUs` from measured capture timing when known. The importer supports a constant interval only; use an explicit manifest for variable frame timing. It does not add ground truth, interpolate frames, or grant safety permission. If import fails after conversion starts, the new directory can contain partial output. Use a new destination for the next attempt.

The converted images and report are local build artifacts under `target`. They are excluded from Git. Preserve them separately if required.
