import { describe, expect, it } from "vitest";
import { retryWhileLocked } from "./retry-locked.mjs";

const locked = () =>
  Object.assign(new Error("Command failed: powershell.exe"), {
    stderr:
      "The process cannot access the file 'node.exe' because it is being used by another process.",
  });

describe("retryWhileLocked", () => {
  it("retries a locked file until the archive succeeds", async () => {
    const waits: number[] = [];
    let calls = 0;
    const result = await retryWhileLocked(
      () => {
        calls++;
        if (calls < 3) throw locked();
        return "zipped";
      },
      { wait: async (ms: number) => void waits.push(ms) },
    );
    expect(result).toBe("zipped");
    expect(calls).toBe(3);
    expect(waits).toEqual([1000, 2000]);
  });

  it("gives up after the last attempt and keeps the original error", async () => {
    let calls = 0;
    await expect(
      retryWhileLocked(
        () => {
          calls++;
          throw locked();
        },
        { attempts: 3, wait: async () => {} },
      ),
    ).rejects.toThrow("Command failed: powershell.exe");
    expect(calls).toBe(3);
  });

  it("does not retry other failures", async () => {
    let calls = 0;
    await expect(
      retryWhileLocked(
        () => {
          calls++;
          throw new Error("Access to the path is denied.");
        },
        { wait: async () => {} },
      ),
    ).rejects.toThrow("Access to the path is denied.");
    expect(calls).toBe(1);
  });
});
