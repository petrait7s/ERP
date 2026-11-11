#!/usr/bin/env python3
"""Write a 256x256 PNG app icon (teal tile with a white W) with no third-party libraries."""
import struct, sys, zlib
N = 256
def inside_round(x, y, r=48):
    cx = min(max(x, r), N - 1 - r); cy = min(max(y, r), N - 1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r
def on_w(x, y):
    # W drawn as four thick strokes
    def seg(x0, y0, x1, y1, w=15):
        dx, dy = x1 - x0, y1 - y0; l2 = dx * dx + dy * dy
        t = max(0, min(1, ((x - x0) * dx + (y - y0) * dy) / l2))
        px, py = x0 + t * dx, y0 + t * dy
        return (x - px) ** 2 + (y - py) ** 2 <= w * w
    return seg(56, 70, 90, 190) or seg(90, 190, 128, 100) or seg(128, 100, 166, 190) or seg(166, 190, 200, 70)
rows = []
for y in range(N):
    row = bytearray([0])
    for x in range(N):
        if not inside_round(x, y): row += bytes([0, 0, 0, 0]); continue
        if on_w(x, y): row += bytes([247, 249, 252, 255]); continue
        t = (x + y) / (2 * N)
        row += bytes([int(15 + t * 20), int(23 + (1 - t) * 20 + t * 60), int(42 + t * 40), 255])
    rows.append(bytes(row))
def chunk(tag, data): c = struct.pack(">I", len(data)) + tag + data; return c + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", N, N, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"".join(rows), 9)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
