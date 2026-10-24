import { beforeEach, describe, expect, it, vi } from "vitest";

// A JPEG decodes to a 2×2 image; the dither and PNG encoding stay real.
const files = vi.hoisted(() => ({
  existing: new Set<string>(),
  write: vi.fn<(content: Uint8Array) => Promise<void>>(),
}));

vi.mock("expo-file-system", () => {
  class Directory {
    uri: string;
    constructor(...parts: unknown[]) {
      this.uri = parts.map((part) => (part as { uri?: string }).uri ?? part).join("/");
    }
    get exists() {
      return true;
    }
  }
  class File {
    uri: string;
    constructor(...parts: unknown[]) {
      this.uri = parts.map((part) => (part as { uri?: string }).uri ?? part).join("/");
    }
    get exists() {
      return files.existing.has(this.uri);
    }
    async bytes() {
      return new Uint8Array([0xff, 0xd8]);
    }
    create() {}
    write(content: Uint8Array) {
      return files.write(content);
    }
  }
  return { Directory, File, Paths: { document: "doc", cache: "cache" } };
});
vi.mock("expo-image-manipulator", () => ({ ImageManipulator: {}, SaveFormat: {} }));
vi.mock("react-native", () => ({ Dimensions: { get: () => ({ width: 1, height: 2 }) } }));
vi.mock("jpeg-js", () => ({
  decode: () => ({ width: 2, height: 2, data: new Uint8Array(16).fill(200) }),
}));

import { renderDither } from "@/features/background/render";

describe("background render", () => {
  beforeEach(() => {
    files.existing.clear();
    files.write.mockReset();
  });

  it("hands back the render's URI only once its PNG is written", async () => {
    // expo-file-system 58 made File.write asynchronous. Resolving early would
    // point the background at a file that is still empty.
    let finish!: () => void;
    files.write.mockReturnValue(new Promise<void>((resolve) => (finish = resolve)));
    let uri: string | undefined;
    const rendering = renderDither("photo.jpg", "bg").then((value) => (uri = value));

    await vi.waitFor(() => expect(files.write).toHaveBeenCalledOnce());
    await Promise.resolve();
    expect(uri).toBeUndefined();

    finish();
    await rendering;
    expect(uri).toBe("cache/background-dither/bg.clear.png");
    const png = files.write.mock.calls[0][0];
    expect([...png.slice(1, 4)].map((byte) => String.fromCharCode(byte)).join("")).toBe("PNG");
  });

  it("fails the render when writing its PNG fails", async () => {
    files.write.mockRejectedValue(new Error("disk full"));
    await expect(renderDither("photo.jpg", "bg")).rejects.toThrow("disk full");
  });

  it("reuses a render that is already on disk without writing it again", async () => {
    files.existing.add("cache/background-dither/bg.clear.png");
    await expect(renderDither("photo.jpg", "bg")).resolves.toBe(
      "cache/background-dither/bg.clear.png",
    );
    expect(files.write).not.toHaveBeenCalled();
  });
});
