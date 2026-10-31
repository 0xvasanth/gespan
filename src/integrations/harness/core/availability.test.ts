import { describe, expect, it, vi } from "vitest";

const missing = vi.hoisted(
  () => () => Promise.reject(new Error("not found")),
);
vi.mock("../../../../host/core/integrations/harness/core/child", () => ({
  resolveAntigravityBinary: missing,
  resolveClaudeBinary: missing,
  resolveCodexBinary: missing,
  resolveCursorBinary: missing,
  resolveDevinBinary: missing,
  resolveFxBinary: missing,
  resolveGrokBinary: missing,
  resolveHermesBinary: missing,
  resolveKiroBinary: missing,
  resolveOmpBinary: missing,
  resolveOpenCodeBinary: missing,
  resolvePiBinary: missing,
}));
vi.mock("./registry", () => ({ isLiveHarness: () => true }));
vi.mock("../../../../host/core/platform/tauri/platform", () => ({ IS_WIN: false }));

import {
  hasProbedHarnessAvailability,
  probeHarnessAvailability,
  subscribeHarnessAvailability,
} from "./availability";

describe("probeHarnessAvailability", () => {
  it("tells subscribers only once the probe counts as finished", async () => {
    // A subscriber re-reading on the change must see the probe as done, or a
    // "no coding agent" state found by this probe would never render.
    const seen: boolean[] = [];
    const stop = subscribeHarnessAvailability(() =>
      seen.push(hasProbedHarnessAvailability()),
    );
    await probeHarnessAvailability();
    stop();
    expect(seen.length).toBeGreaterThan(0);
    expect(seen.every(Boolean)).toBe(true);
  });
});
