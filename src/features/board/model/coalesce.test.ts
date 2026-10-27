import { describe, expect, it } from "vitest";
import { coalesce } from "./coalesce";

/** A run that waits for the test to finish it. */
function gatedRun() {
  const pending: { done: () => void; fail: (cause: unknown) => void }[] = [];
  let calls = 0;
  const run = () => {
    calls += 1;
    return new Promise<void>((resolve, reject) => {
      pending.push({ done: resolve, fail: reject });
    });
  };
  return { run, pending, calls: () => calls };
}

const tick = async () => {
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
};

describe("coalesce", () => {
  it("a burst while a run is in flight costs one trailing run, not one each", async () => {
    const gated = gatedRun();
    const reload = coalesce(gated.run);
    void reload();
    for (let i = 0; i < 49; i += 1) void reload();
    expect(gated.calls()).toBe(1);
    gated.pending.shift()!.done();
    await tick();
    // The trailing run starts after the burst, once.
    expect(gated.calls()).toBe(2);
    gated.pending.shift()!.done();
    await tick();
    expect(gated.calls()).toBe(2);
  });

  it("a call with nothing in flight runs at once and none trails", async () => {
    const gated = gatedRun();
    const reload = coalesce(gated.run);
    const first = reload();
    gated.pending.shift()!.done();
    await first;
    expect(gated.calls()).toBe(1);
    const second = reload();
    expect(gated.calls()).toBe(2);
    gated.pending.shift()!.done();
    await second;
    expect(gated.calls()).toBe(2);
  });

  it("calls during the trailing run get one more run after it", async () => {
    const gated = gatedRun();
    const reload = coalesce(gated.run);
    void reload();
    void reload();
    gated.pending.shift()!.done();
    await tick();
    expect(gated.calls()).toBe(2);
    void reload();
    void reload();
    gated.pending.shift()!.done();
    await tick();
    expect(gated.calls()).toBe(3);
    gated.pending.shift()!.done();
    await tick();
    expect(gated.calls()).toBe(3);
  });

  it("a failed run still lets the trailing run happen, and later calls run", async () => {
    const gated = gatedRun();
    const reload = coalesce(gated.run);
    const joined = reload();
    void reload();
    gated.pending.shift()!.fail(new Error("locked"));
    await tick();
    expect(gated.calls()).toBe(2);
    gated.pending.shift()!.done();
    await expect(joined).resolves.toBeUndefined();
    void reload();
    expect(gated.calls()).toBe(3);
    gated.pending.shift()!.done();
  });
});
