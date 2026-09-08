#!/usr/bin/env python3
"""Draw the AgentToast mark as a PNG, at any size, with no dependencies.

The mark is the same one `scripts/generate-icons.ps1` draws for Windows — a
mid-tone indigo card with light bars — because it is the same application and
the icon is how people find it. What differs is only how it gets rendered:
there is no System.Drawing on a Mac, and a macOS icon has to go up to 1024px,
where upscaling the existing 256px PNG would be visibly soft.

Nothing here is imported from outside the standard library. A contributor with a
Mac and Xcode's command line tools — which they need for Rust anyway — can
regenerate the icons, with no `pip install` in the way.

Anti-aliasing is done by spans rather than by point sampling. A rounded
rectangle is convex, so every scanline through it is a single interval; the
exact horizontal overlap of that interval with each pixel is arithmetic, and
only the vertical direction is supersampled. That is what makes a 1024px render
finish instantly instead of evaluating sixteen million sample points.

Usage:  render-icon.py <size> <output.png>
"""

import math
import struct
import sys
import zlib

# Accent indigo, between the light and dark theme accents so one mark works on
# both. The bars are near-white for maximum contrast against it.
CARD = (91, 110, 225)
BAR = (245, 246, 252)

# Sub-scanlines per output row. Eight is past the point where more of them
# changes a byte of the output.
OVERSAMPLE = 8


def rounded_span(y, x0, y0, w, h, r):
    """Horizontal interval of a rounded rectangle at height `y`, or None."""
    r = min(r, w / 2, h / 2)

    if y < y0 or y > y0 + h:
        return None

    # The straight middle: the full width.
    if y0 + r <= y <= y0 + h - r:
        return (x0, x0 + w)

    # A corner band: the width narrows by the circle's chord.
    dy = (y0 + r) - y if y < y0 + r else y - (y0 + h - r)
    if dy >= r:
        return None
    dx = math.sqrt(r * r - dy * dy)
    return (x0 + r - dx, x0 + w - r + dx)


def add_span(coverage, span, weight):
    """Accumulate an interval's per-pixel overlap into a row of coverages."""
    if span is None:
        return
    a, b = span
    first = max(0, int(math.floor(a)))
    last = min(len(coverage) - 1, int(math.ceil(b)) - 1)
    for x in range(first, last + 1):
        overlap = min(b, x + 1) - max(a, x)
        if overlap > 0:
            coverage[x] += overlap * weight


def shapes(size):
    """The card, and the bars drawn on top of it, at a given icon size.

    Below 20px the two bars turn to mush, so one bolder bar is drawn instead —
    the same threshold and the same proportions as the Windows script, so the
    two platforms show the same mark rather than two that merely rhyme.
    """
    pad = size * 0.06
    side = size - pad * 2
    radius = max(2.0, size * 0.22)
    card = (pad, pad, side, side, radius)

    left = size * 0.28
    width = size * 0.44

    if size >= 20:
        height = max(1.0, size * 0.09)
        bars = [
            (left, size * 0.34, width, height, height / 2),
            (left, size * 0.55, width * 0.62, height, height / 2),
        ]
    else:
        height = max(2.0, size * 0.16)
        bars = [(left, (size - height) / 2, width, height, height / 2)]

    return card, bars


def render(size):
    """RGBA bytes for the mark at `size` x `size`."""
    card, bars = shapes(size)
    out = bytearray()

    for row in range(size):
        card_cov = [0.0] * size
        bar_cov = [0.0] * size

        for sub in range(OVERSAMPLE):
            y = row + (sub + 0.5) / OVERSAMPLE
            add_span(card_cov, rounded_span(y, *card), 1.0 / OVERSAMPLE)
            for bar in bars:
                add_span(bar_cov, rounded_span(y, *bar), 1.0 / OVERSAMPLE)

        for x in range(size):
            alpha = min(1.0, card_cov[x])
            if alpha <= 0.0:
                out += b"\x00\x00\x00\x00"
                continue

            # The bars sit on the card, so what is left of the card is whatever
            # they do not cover.
            over = min(bar_cov[x], alpha)
            under = alpha - over
            colour = tuple(
                round((BAR[c] * over + CARD[c] * under) / alpha) for c in range(3)
            )
            out += bytes(colour) + bytes([round(alpha * 255)])

    return bytes(out)


def write_png(path, size, rgba):
    """Write 8-bit RGBA as a PNG. Three chunks is the whole format."""

    def chunk(kind, payload):
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    stride = size * 4
    # Filter type 0 (none) in front of every scanline.
    raw = b"".join(
        b"\x00" + rgba[y * stride : (y + 1) * stride] for y in range(size)
    )

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )

    with open(path, "wb") as handle:
        handle.write(png)


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__.strip().splitlines()[-1])

    size = int(sys.argv[1])
    path = sys.argv[2]
    write_png(path, size, render(size))
    print(f"wrote {path} ({size}x{size})")


if __name__ == "__main__":
    main()
