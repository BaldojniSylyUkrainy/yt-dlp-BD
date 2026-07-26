#!/usr/bin/env python3
"""Build the native app icon master from the existing three-hand artwork."""

from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "src/assets/logo-call-me-hands.png"
OUTPUT = ROOT / "src/assets/app-icon-call-me-hands-rounded.png"

CANVAS_SIZE = 1024
RENDER_SCALE = 4
TILE_SIZE = 868
SUPERELLIPSE_POWER = 4.4
HAND_WIDTH = 700


def superellipse_polygon(center: float, radius: float, power: float) -> list[tuple[float, float]]:
    points: list[tuple[float, float]] = []
    for step in range(1440):
        angle = math.tau * step / 1440
        cosine = math.cos(angle)
        sine = math.sin(angle)
        x = center + radius * math.copysign(abs(cosine) ** (2 / power), cosine)
        y = center + radius * math.copysign(abs(sine) ** (2 / power), sine)
        points.append((x, y))
    return points


def build_icon() -> None:
    size = CANVAS_SIZE * RENDER_SCALE
    tile_size = TILE_SIZE * RENDER_SCALE
    tile_offset = (size - tile_size) // 2

    mask = Image.new("L", (size, size), 0)
    mask_draw = ImageDraw.Draw(mask)
    center = size / 2
    mask_draw.polygon(
        superellipse_polygon(center, tile_size / 2, SUPERELLIPSE_POWER),
        fill=255,
    )

    tile = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    pixels = tile.load()
    for y in range(tile_offset, tile_offset + tile_size):
        progress = (y - tile_offset) / max(1, tile_size - 1)
        red = round(25 + (13 - 25) * progress)
        green = round(22 + (12 - 22) * progress)
        blue = round(25 + (15 - 25) * progress)
        for x in range(tile_offset, tile_offset + tile_size):
            pixels[x, y] = (red, green, blue, 255)
    tile.putalpha(mask)

    hands = Image.open(SOURCE).convert("RGBA")
    alpha_bounds = hands.getchannel("A").getbbox()
    if alpha_bounds:
        hands = hands.crop(alpha_bounds)
    hand_width = HAND_WIDTH * RENDER_SCALE
    hand_height = round(hands.height * hand_width / hands.width)
    hands = hands.resize((hand_width, hand_height), Image.Resampling.LANCZOS)
    hand_position = ((size - hand_width) // 2, (size - hand_height) // 2 + 14 * RENDER_SCALE)
    tile.alpha_composite(hands, hand_position)

    tile.resize((CANVAS_SIZE, CANVAS_SIZE), Image.Resampling.LANCZOS).save(OUTPUT)


if __name__ == "__main__":
    build_icon()
