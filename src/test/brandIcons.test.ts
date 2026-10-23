import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const root = join(__dirname, "..", "..");
const read = (rel: string) => readFileSync(join(root, rel), "utf8");

// The icon's blocks run clockwise from the left-top block, from near-white down
// to deep purple. These are the colours of the original 1024px app icon.
const RAMP = [
  "#ede9fe",
  "#c0affc",
  "#af97fa",
  "#9f7ef8",
  "#8f64f6",
  "#7e47ea",
  "#6e2ada",
  "#5b22b4",
];
const BLOCK_IDS = [
  "left-top",
  "top-left",
  "top-right",
  "right-top",
  "right-bottom",
  "bottom-right",
  "bottom-left",
  "left-bottom",
];
const BLOCK_SIZE = 168;

type Rect = Record<string, string>;

function rects(svg: string): Rect[] {
  return [...svg.matchAll(/<rect\b([^>]*?)\/?>/g)].map((m) =>
    Object.fromEntries(
      [...m[1].matchAll(/([\w-]+)="([^"]*)"/g)].map((a) => [a[1], a[2]]),
    ),
  );
}

function blocks(svg: string): Map<string, Rect> {
  return new Map(
    rects(svg)
      .filter((r) => r.id)
      .map((r) => [r.id, r]),
  );
}

const luma = (hex: string) => {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const shade = (hex: string, factor: number) =>
  "#" +
  [1, 3, 5]
    .map((i) =>
      Math.floor(parseInt(hex.slice(i, i + 2), 16) * factor)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("");

describe("the colour ramp", () => {
  it("runs from near-white to deep purple, getting darker at every step", () => {
    const lumas = RAMP.map(luma);
    expect(lumas[0]).toBeGreaterThan(225);
    expect(lumas[lumas.length - 1]).toBeLessThan(70);
    for (let i = 1; i < lumas.length; i += 1) {
      expect(lumas[i]).toBeLessThan(lumas[i - 1]);
    }
  });
});

describe("src-tauri/icons/icon.svg (the app icon)", () => {
  const svg = read("src-tauri/icons/icon.svg");
  const byId = blocks(svg);

  it("has the nine blocks of the design, all one size, with rounded corners", () => {
    expect([...byId.keys()].sort()).toEqual([...BLOCK_IDS, "prompt"].sort());
    for (const r of byId.values()) {
      expect(Number(r.width)).toBe(BLOCK_SIZE);
      expect(Number(r.height)).toBe(BLOCK_SIZE);
      expect(Number(r.rx)).toBeGreaterThan(0);
    }
  });

  it("uses the original ramp, clockwise from the left-top block", () => {
    expect(BLOCK_IDS.map((id) => byId.get(id)!.fill.toLowerCase())).toEqual(
      RAMP,
    );
    expect(byId.get("prompt")!.fill.toLowerCase()).toBe(RAMP[0]);
  });

  it("sits on a full-bleed black tile", () => {
    const background = rects(svg).find((r) => !r.id)!;
    expect(background.fill.toLowerCase()).toBe("#000");
    expect(background.width).toBe("1024");
    expect(background.height).toBe("1024");
  });

  it("draws the prompt as strokes, not font-dependent text", () => {
    expect(svg).not.toMatch(/<text\b/);
    expect(svg).not.toMatch(/font-family/);
    expect(svg.match(/<path\b/g)).toHaveLength(2);
  });

  it("has no stray shapes beyond the tile, the nine blocks and the prompt", () => {
    expect(rects(svg)).toHaveLength(1 + 9);
  });
});

describe("docs/assets/logo.svg (the README mark)", () => {
  const svg = read("docs/assets/logo.svg");
  const byId = blocks(svg);

  it("uses the same ramp and the same nine blocks as the app icon", () => {
    expect(BLOCK_IDS.map((id) => byId.get(id)!.fill.toLowerCase())).toEqual(
      RAMP,
    );
    expect(byId.get("prompt")!.fill.toLowerCase()).toBe(RAMP[0]);
    expect(byId.size).toBe(9);
  });

  it("has a transparent background", () => {
    expect(rects(svg).every((r) => r.id)).toBe(true);
  });

  it("outlines each block in its own darker shade so light blocks show on white", () => {
    for (const [id, r] of byId) {
      const fill = r.fill.toLowerCase();
      expect(r.stroke.toLowerCase(), id).toBe(shade(fill, 0.62));
      expect(luma(r.stroke), id).toBeLessThan(luma(fill));
    }
  });

  it("crops tightly around the mark", () => {
    expect(svg).toMatch(/viewBox="104 104 816 816"/);
  });
});

describe("docs/assets/logo.png (the README image)", () => {
  const png = readFileSync(join(root, "docs/assets/logo.png"));

  it("is a 512x512 PNG with an alpha channel", () => {
    expect(png.subarray(1, 4).toString("ascii")).toBe("PNG");
    expect(png.readUInt32BE(16)).toBe(512); // IHDR width
    expect(png.readUInt32BE(20)).toBe(512); // IHDR height
    expect(png[25]).toBe(6); // colour type 6: RGBA
  });

  it("is what the README shows", () => {
    expect(read("README.md")).toContain('src="docs/assets/logo.png"');
    expect(existsSync(join(root, "docs/assets/logo.png"))).toBe(true);
  });
});
