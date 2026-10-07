"""Make local grayscale smoke fixtures from downloaded example photographs."""
import argparse
import json
from pathlib import Path

from PIL import Image, ImageOps


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    frames = []
    for index, (filename, hazard) in enumerate([("bus.jpg", "Human"), ("dog.jpg", "Dog")]):
        with Image.open(args.directory / filename) as image:
            mono = ImageOps.pad(image.convert("L"), (640, 480), color=0)
        name = Path(filename).stem + ".pgm"
        # Write the strict three-line PGM header required by ImageSequence.
        (args.directory / name).write_bytes(b"P5\n640 480\n255\n" + mono.tobytes())
        frames.append({"file": name, "id": index, "timestamp": index * 10_000,
                       "truth": {"targets": [], "hazard": hazard}})
    (args.directory / "real-images.json").write_text(json.dumps({"version": 1, "size": [640, 480], "frames": frames}), encoding="utf-8")


if __name__ == "__main__":
    main()
