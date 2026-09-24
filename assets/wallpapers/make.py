#!/usr/bin/env python3
"""Makes qdesk's own wallpapers: soft abstract shapes in the colours of the Quvyta themes.

Run it from anywhere; it writes ember.png, dusk.png and tide.png beside itself. Only the
standard library is used, so the pictures can be made again on any machine and come out the
same, byte for byte.
"""

import math
import os
import struct
import zlib

# Enough for a terminal of 384 columns and 108 rows, where every cell shows two pixels; a larger
# screen stretches it a little, which a soft picture does not show.
WIDTH, HEIGHT = 384, 216


def hex_colour(text):
    text = text.lstrip("#")
    return tuple(int(text[i:i + 2], 16) / 255 for i in (0, 2, 4))


def mix(a, b, share):
    return tuple(x + (y - x) * share for x, y in zip(a, b))


def glow(x, y, cx, cy, rx, ry):
    """How much of a soft round light at cx, cy reaches x, y: 1 at its heart, 0 far away."""
    d = ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2
    return math.exp(-d * 2.2)


def ember(x, y):
    # Amber: the warm dark ground, a low sun of the accent, a paler light above it.
    canvas, surface, accent, pale = map(hex_colour, ("#12100E", "#2A241D", "#F59E0B", "#FED7AA"))
    colour = mix(canvas, surface, y * 0.9)
    colour = mix(colour, accent, 0.62 * glow(x, y, 0.70, 1.02, 0.55, 0.50))
    colour = mix(colour, pale, 0.22 * glow(x, y, 0.72, 0.88, 0.16, 0.14))
    colour = mix(colour, accent, 0.10 * glow(x, y, 0.18, 0.20, 0.40, 0.35))
    return colour


def dusk(x, y):
    # Iris: an indigo sky darkening upwards, two soft violet clouds drifting across it.
    canvas, raised, accent, pale = map(hex_colour, ("#0E0F18", "#202336", "#818CF8", "#C7D2FE"))
    colour = mix(canvas, raised, y)
    colour = mix(colour, accent, 0.45 * glow(x, y, 0.28, 0.72, 0.42, 0.30))
    colour = mix(colour, pale, 0.18 * glow(x, y, 0.78, 0.30, 0.30, 0.22))
    colour = mix(colour, accent, 0.20 * glow(x, y, 0.95, 0.95, 0.35, 0.30))
    return colour


def tide(x, y):
    # Nordic: deep water, three slow waves of the accent rising towards the bottom.
    canvas, surface, accent, pale = map(hex_colour, ("#0B1118", "#192637", "#38BDF8", "#A5E4FD"))
    colour = mix(canvas, surface, y * 0.8)
    for n, (height, amount, tone) in enumerate(((0.58, 0.16, accent), (0.72, 0.24, accent), (0.86, 0.14, pale))):
        crest = height + 0.05 * math.sin(x * math.pi * 2 * (1.0 + n * 0.35) + n * 1.7)
        below = 1 / (1 + math.exp(-(y - crest) * 28))
        colour = mix(colour, tone, amount * below)
    return colour


def png(path, paint):
    rows = []
    for row in range(HEIGHT):
        line = bytearray()
        for column in range(WIDTH):
            r, g, b = paint(column / (WIDTH - 1), row / (HEIGHT - 1))
            line += bytes(max(0, min(255, round(c * 255))) for c in (r, g, b))
        # The "sub" filter: every byte as its difference from the same colour of the pixel to its
        # left, which a smooth picture turns into long runs of small numbers.
        filtered = bytes((line[i] - (line[i - 3] if i >= 3 else 0)) & 0xFF for i in range(len(line)))
        rows.append(b"\x01" + filtered)
    data = zlib.compress(b"".join(rows), 9)

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    header = struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8, 2, 0, 0, 0)
    with open(path, "wb") as out:
        out.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", data) + chunk(b"IEND", b""))


if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    for name, paint in (("ember", ember), ("dusk", dusk), ("tide", tide)):
        png(os.path.join(here, f"{name}.png"), paint)
