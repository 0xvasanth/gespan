import { describe, expect, it } from "vitest";
import type { LinkExtra } from "./linkStatus";
import { extraKey } from "./linkStatus";
import {
  LINK_STATUS_CONCURRENCY,
  lookupLinkStatuses,
  mapLimited,
  type LinkStatusCache,
  type StatusLink,
} from "./linkStatusLookup";

const tick = async () => {
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
};

/** Reads that wait for the test, tracking how many are running at once. */
function gatedReads() {
  const waiting: { link: StatusLink; done: (value: LinkExtra) => void }[] = [];
  let running = 0;
  let peak = 0;
  const read = (link: StatusLink) => {
    running += 1;
    peak = Math.max(peak, running);
    return new Promise<LinkExtra>((resolve) => {
      waiting.push({
        link,
        done: (value) => {
          running -= 1;
          resolve(value);
        },
      });
    });
  };
  return { read, waiting, peak: () => peak, running: () => running };
}

const pr = (n: number): StatusLink => ({
  kind: "pr",
  reference: `https://github.com/o/r/pull/${n}`,
});
const worktree = (n: number): StatusLink => ({
  kind: "worktree",
  reference: `/work/wt${n}`,
});

describe("lookupLinkStatuses", () => {
  it("reads at most four links at once however many the board holds", async () => {
    expect(LINK_STATUS_CONCURRENCY).toBe(4);
    const links = [
      ...Array.from({ length: 10 }, (_, i) => pr(i)),
      ...Array.from({ length: 10 }, (_, i) => worktree(i)),
    ];
    const gated = gatedReads();
    const result = lookupLinkStatuses(links, {
      read: gated.read,
      cache: new Map(),
    });
    await tick();
    expect(gated.running()).toBe(4);
    // Each finished read lets exactly one more start.
    while (gated.waiting.length) {
      gated.waiting.shift()!.done({ state: "idle" });
      await tick();
      expect(gated.running()).toBeLessThanOrEqual(4);
    }
    const out = await result;
    expect(gated.peak()).toBe(4);
    expect(Object.keys(out)).toHaveLength(20);
    expect(out[extraKey(pr(7))]).toEqual({ state: "idle" });
  });

  it("answers a link read within the last few seconds from its last result", async () => {
    let clock = 1_000;
    const now = () => clock;
    const cache: LinkStatusCache = new Map();
    const reads: string[] = [];
    const read = async (link: StatusLink): Promise<LinkExtra> => {
      reads.push(link.reference);
      return { state: link.kind === "pr" ? "open" : "idle" };
    };
    const first = await lookupLinkStatuses([pr(1), worktree(1)], {
      read,
      cache,
      now,
    });
    expect(reads).toHaveLength(2);

    clock += 3_000;
    const again = await lookupLinkStatuses([pr(1), worktree(1), pr(2)], {
      read,
      cache,
      now,
    });
    // Only the new link is read; the fresh ones answer from the cache.
    expect(reads).toEqual([pr(1).reference, worktree(1).reference, pr(2).reference]);
    expect(again[extraKey(pr(1))]).toEqual(first[extraKey(pr(1))]);
    expect(again[extraKey(worktree(1))]).toEqual({ state: "idle" });

    // Past the fresh window every link is read again.
    clock += 5_000;
    await lookupLinkStatuses([pr(1), worktree(1)], { read, cache, now });
    expect(reads.slice(3)).toEqual([pr(1).reference, worktree(1).reference]);
  });

  it("reads a link listed twice once", async () => {
    const reads: string[] = [];
    await lookupLinkStatuses([pr(1), pr(1)], {
      read: async (link) => {
        reads.push(link.reference);
        return {};
      },
      cache: new Map(),
    });
    expect(reads).toEqual([pr(1).reference]);
  });
});

describe("mapLimited", () => {
  it("keeps the input order and handles an empty list", async () => {
    expect(await mapLimited([], 4, async (n: number) => n)).toEqual([]);
    const out = await mapLimited([3, 1, 2], 2, async (n) => {
      await new Promise((resolve) => setTimeout(resolve, n));
      return n * 10;
    });
    expect(out).toEqual([30, 10, 20]);
  });
});
