"""Native ncnn prototype worker. Stdout is a fixed-size binary protocol only."""
import argparse
import struct
import sys

import ncnn
import numpy as np


def read_exact(stream, size):
    data = bytearray()
    while len(data) < size:
        part = stream.read(size - len(data))
        if not part:
            raise EOFError("truncated request")
        data.extend(part)
    return bytes(data)


def hazard_scores(net, pixels, width, height):
    scale = 416 / max(width, height)
    w, h = (416, int(height * scale)) if width > height else (int(width * scale), 416)
    if min(w, h) < 1:
        raise ValueError("unsupported aspect ratio")
    # Replicate monochrome into BGR. No fixture labels enter this process.
    image = np.repeat(np.frombuffer(pixels, dtype=np.uint8).reshape(height, width, 1), 3, axis=2)
    tensor = ncnn.Mat.from_pixels_resize(image, ncnn.Mat.PixelType.PIXEL_BGR, width, height, w, h)
    wp, hp = (-w) % 32, (-h) % 32
    padded = ncnn.copy_make_border(tensor, hp // 2, hp - hp // 2, wp // 2, wp - wp // 2, ncnn.BorderType.BORDER_CONSTANT, 0.0)
    padded.substract_mean_normalize([103.53, 116.28, 123.675], [1 / 57.375, 1 / 57.12, 1 / 58.395])
    maxima = np.zeros(3, dtype=np.float32)
    with net.create_extractor() as extractor:
        if extractor.input("in0", padded) != 0:
            raise RuntimeError("input failed")
        for name, stride in [("231", 8), ("228", 16), ("225", 32), ("222", 64)]:
            status, output = extractor.extract(name)
            if status != 0:
                raise RuntimeError("extraction failed: " + name)
            values = np.asarray(output)
            # Pinned model: 80 class logits + four distributions of eight bins.
            expected = (112, (h + hp + stride - 1) // stride, (w + wp + stride - 1) // stride)
            if values.shape != expected or not np.isfinite(values).all():
                raise ValueError("invalid tensor: " + name + " " + str(values.shape))
            # Examine all protected classes, before argmax or NMS can discard them.
            logits = values[[0, 16, 15]].max(axis=(1, 2))
            probabilities = 1 / (1 + np.exp(-np.clip(logits, -80, 80)))
            maxima = np.maximum(maxima, probabilities)
    return maxima.tolist()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--param", required=True)
    parser.add_argument("--weights", required=True)
    parser.add_argument("--threads", type=int, required=True, choices=range(1, 5))
    args = parser.parse_args()
    with ncnn.Net() as net:
        net.opt.use_vulkan_compute = False
        net.opt.num_threads = args.threads
        if net.load_param(args.param) != 0 or net.load_model(args.weights) != 0:
            raise RuntimeError("model load failed")
        while True:
            first = sys.stdin.buffer.read(1)
            if not first:
                return
            header = first + read_exact(sys.stdin.buffer, 27)
            magic, width, height, frame, timestamp = struct.unpack("<4sIIQQ", header)
            if magic != b"ND01" or min(width, height) == 0 or width * height > 16_777_216:
                raise ValueError("invalid request")
            pixels = read_exact(sys.stdin.buffer, width * height)
            scores = hazard_scores(net, pixels, width, height)
            sys.stdout.buffer.write(struct.pack("<4sQQfff", magic, frame, timestamp, *scores))
            sys.stdout.buffer.flush()


if __name__ == "__main__":
    main()
