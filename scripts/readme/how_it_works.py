# /// script
# requires-python = ">=3.10"
# dependencies = ["playwright>=1.48", "pillow>=10", "numpy>=1.26"]
# ///
"""Render the README "How it works" animation to GIFs.

The scene lives in how-it-works.html next to this script: one loop, four
parts (Board, Sessions, Automations, Pilots), drawn as a pure function of time.
Open that file in a browser for the interactive version with play/pause, a
scrubber and chapters. This script steps it frame by frame in headless Chrome
and writes docs/assets/how-it-works-{dark,light}.gif, since GitHub READMEs do
not run scripts.

    uv run scripts/readme/how_it_works.py
    uv run scripts/readme/how_it_works.py --theme dark --frames 0,60,120  # PNG stills
"""

from __future__ import annotations

import argparse
import io
from pathlib import Path

import numpy as np
from PIL import Image
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SCENE = HERE / "how-it-works.html"
ASSETS = ROOT / "docs" / "assets"

FPS = 20
SCALE = 1.2  # device pixels per CSS pixel: 1040x660 -> 1248x792
COLOURS = 256


def capture(theme: str, frames: list[int] | None = None) -> list[Image.Image]:
    with sync_playwright() as p:
        try:
            browser = p.chromium.launch(channel="chrome")
        except Exception:
            browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 1040, "height": 660}, device_scale_factor=SCALE)
        page.goto(f"{SCENE.as_uri()}?capture&theme={theme}")
        page.wait_for_function("window.render !== undefined")
        page.evaluate("document.fonts.ready")
        seconds = page.evaluate("window.LOOP_SECONDS")
        stage = page.locator("#stage")
        images = []
        for f in frames if frames is not None else range(round(seconds * FPS)):
            page.evaluate(f"window.render({f / FPS})")
            images.append(Image.open(io.BytesIO(stage.screenshot(animations="disabled"))).convert("RGB"))
        browser.close()
    return images


def nearest(colours: np.ndarray, palette: np.ndarray) -> np.ndarray:
    """Index of the closest palette entry for each colour, in chunks."""
    out = np.empty(len(colours), dtype=np.uint8)
    for i in range(0, len(colours), 8192):
        chunk = colours[i : i + 8192, None, :] - palette[None, :, :]
        out[i : i + 8192] = np.einsum("ijk,ijk->ij", chunk, chunk).argmin(axis=1)
    return out


def build_palette(colours: np.ndarray, counts: np.ndarray) -> np.ndarray:
    """Weighted k-means over every distinct colour in the loop.

    Median cut gives the big flat surfaces most of the palette, so small
    accents (agent badges, status pills, window buttons) came out the wrong
    hue. Weighting each colour by the square root of its pixel count keeps
    those accents while still favouring what covers the most screen.
    """
    pts = colours.astype(np.float32)
    weight = np.sqrt(counts).astype(np.float32)
    rng = np.random.default_rng(0)
    centres = pts[rng.choice(len(pts), COLOURS, replace=False, p=weight / weight.sum())]
    for _ in range(16):
        label = nearest(pts, centres)
        total = np.zeros_like(centres)
        np.add.at(total, label, pts * weight[:, None])
        mass = np.bincount(label, weights=weight, minlength=COLOURS)[:, None]
        used = mass[:, 0] > 0
        centres[used] = total[used] / mass[used]
    return np.clip(np.rint(centres), 0, 255).astype(np.float32)


def pack(rgb: np.ndarray) -> np.ndarray:
    rgb = rgb.astype(np.int32)
    return (rgb[..., 0] << 16) | (rgb[..., 1] << 8) | rgb[..., 2]


def unpack(keys: np.ndarray) -> np.ndarray:
    return np.stack([(keys >> 16) & 255, (keys >> 8) & 255, keys & 255], axis=-1).astype(np.float32)


def write_gif(images: list[Image.Image], path: Path) -> None:
    # One palette for the whole loop so surfaces do not flicker, and no
    # dithering so still areas stay still between frames.
    arrays = [np.asarray(im) for im in images]
    keys = [pack(a) for a in arrays]
    unique, counts = np.unique(np.concatenate([k.ravel() for k in keys[::4]]), return_counts=True)
    every = np.unique(np.concatenate([k.ravel() for k in keys]))
    palette = build_palette(unpack(unique), counts)
    lookup = nearest(unpack(every), palette)
    flat = palette.astype(np.uint8).ravel().tolist()
    frames = []
    for k in keys:
        frame = Image.fromarray(lookup[np.searchsorted(every, k)], "P")
        frame.putpalette(flat)
        frames.append(frame)
    frames[0].save(path, save_all=True, append_images=frames[1:], duration=round(1000 / FPS),
                   loop=0, optimize=True, disposal=1)
    print(f"wrote {path.relative_to(ROOT)} ({path.stat().st_size / 1024:.0f} KB, {len(frames)} frames)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--theme", choices=["dark", "light"], help="only this theme")
    parser.add_argument("--frames", help="comma-separated frame numbers to save as PNG stills instead")
    parser.add_argument("--out", type=Path, default=ASSETS, help="output directory")
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    for theme in [args.theme] if args.theme else ["dark", "light"]:
        if args.frames:
            numbers = [int(n) for n in args.frames.split(",")]
            for n, im in zip(numbers, capture(theme, numbers)):
                im.save(args.out / f"how-it-works-{theme}-{n:03d}.png")
            print(f"wrote {len(numbers)} stills to {args.out}")
        else:
            write_gif(capture(theme), args.out / f"how-it-works-{theme}.gif")


if __name__ == "__main__":
    main()
