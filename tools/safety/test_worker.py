"""Protocol test double. Never use as an image detector."""
import argparse
import struct
import sys
import time

parser = argparse.ArgumentParser()
parser.add_argument("--param")
parser.add_argument("--weights")
parser.add_argument("--threads")
args = parser.parse_args()
while True:
    header = sys.stdin.buffer.read(28)
    if not header:
        break
    magic, w, h, frame, timestamp = struct.unpack("<4sIIQQ", header)
    sys.stdin.buffer.read(w * h)
    if args.param == "timeout":
        time.sleep(30)
    if args.param == "crash":
        sys.exit(1)
    if args.param == "mismatch":
        frame += 1
    scores = [0.0, 0.0, 0.0]
    if args.param in ["human", "dog", "cat"]:
        scores[["human", "dog", "cat"].index(args.param)] = 0.8
    sys.stdout.buffer.write(struct.pack("<4sQQfff", magic, frame, timestamp, *scores))
    sys.stdout.buffer.flush()
