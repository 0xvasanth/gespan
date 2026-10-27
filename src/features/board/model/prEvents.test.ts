import { describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { onLinkedWork } from "./autoMove";
import type { LinkedWorkEvent } from "./board";
import { createPrEvents, prEvent } from "./prEvents";

const A = "https://github.com/acme/app/pull/1";
const B = "https://github.com/acme/app/pull/2";

function setup(
  notify: (reference: string, event: LinkedWorkEvent) => Promise<void> = () =>
    Promise.resolve(),
) {
  const calls: [string, string, LinkedWorkEvent][] = [];
  const report = vi.fn();
  const events = createPrEvents((kind, reference, event) => {
    calls.push([kind, reference, event]);
    return notify(reference, event);
  }, report);
  return { events, calls, report };
}

const flush = async () => {
  for (let i = 0; i < 3; i += 1) await Promise.resolve();
};

describe("prEvent", () => {
  it("maps an open PR to pr_opened and a merged one to pr_merged", () => {
    expect(prEvent("open")).toBe("pr_opened");
    expect(prEvent("merged")).toBe("pr_merged");
  });

  it.each(["closed", "unknown", undefined] as const)(
    "reports nothing for a %s PR",
    (state) => {
      expect(prEvent(state)).toBeNull();
    },
  );
});

describe("createPrEvents", () => {
  it("reports pr_opened when a refresh first sees the PR open, then not again", () => {
    const { events, calls } = setup();
    events.observe([{ reference: A, state: "open" }]);
    events.observe([{ reference: A, state: "open" }]);
    expect(calls).toEqual([["pr", A, "pr_opened"]]);
  });

  it("reports pr_merged when a later refresh sees the merge", () => {
    const { events, calls } = setup();
    events.observe([{ reference: A, state: "open" }]);
    events.observe([{ reference: A, state: "merged" }]);
    events.observe([{ reference: A, state: "merged" }]);
    expect(calls).toEqual([
      ["pr", A, "pr_opened"],
      ["pr", A, "pr_merged"],
    ]);
  });

  it("reports only pr_merged for a PR first seen already merged", () => {
    const { events, calls } = setup();
    events.observe([{ reference: A, state: "merged" }]);
    expect(calls).toEqual([["pr", A, "pr_merged"]]);
  });

  it("reports nothing for a closed or unreadable PR, and keeps what it saw", () => {
    const { events, calls } = setup();
    events.observe([
      { reference: A, state: "closed" },
      { reference: B, state: "unknown" },
    ]);
    expect(calls).toEqual([]);
    events.observe([{ reference: B, state: "open" }]);
    // A refresh that can't read the PR (remote, gh down) is not a change.
    events.observe([{ reference: B, state: "unknown" }]);
    events.observe([{ reference: B, state: "open" }]);
    expect(calls).toEqual([["pr", B, "pr_opened"]]);
  });

  it("tracks each PR on its own", () => {
    const { events, calls } = setup();
    events.observe([
      { reference: A, state: "open" },
      { reference: B, state: "merged" },
    ]);
    events.observe([
      { reference: A, state: "merged" },
      { reference: B, state: "merged" },
    ]);
    expect(calls).toEqual([
      ["pr", A, "pr_opened"],
      ["pr", B, "pr_merged"],
      ["pr", A, "pr_merged"],
    ]);
  });

  it("does not send twice while the first send is still out", () => {
    let release!: () => void;
    const { events, calls } = setup(
      () => new Promise<void>((resolve) => (release = resolve)),
    );
    events.observe([{ reference: A, state: "open" }]);
    events.observe([{ reference: A, state: "open" }]);
    release();
    expect(calls).toHaveLength(1);
  });

  it("reports a failed send and retries it on the next refresh", async () => {
    let fail = true;
    const { events, calls, report } = setup(() =>
      fail ? Promise.reject("database is locked") : Promise.resolve(),
    );
    events.observe([{ reference: A, state: "open" }]);
    await flush();
    expect(report).toHaveBeenCalledWith("database is locked");
    fail = false;
    events.observe([{ reference: A, state: "open" }]);
    await flush();
    events.observe([{ reference: A, state: "open" }]);
    expect(calls).toEqual([
      ["pr", A, "pr_opened"],
      ["pr", A, "pr_opened"],
    ]);
  });

  it("reports a notifier that throws synchronously without throwing", () => {
    const { events, report } = setup(() => {
      throw new Error("boom");
    });
    expect(() => events.observe([{ reference: A, state: "open" }])).not.toThrow();
    expect(report).toHaveBeenCalledWith(new Error("boom"));
  });

  it("sends the PR's events to the store as board_linked_event with kind pr", async () => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
    const report = vi.fn();
    const events = createPrEvents(onLinkedWork, report);
    events.observe([{ reference: A, state: "merged" }]);
    await flush();
    expect(invoke.mock.calls).toEqual([
      ["board_linked_event", { kind: "pr", id: A, event: "pr_merged" }],
    ]);
    expect(report).not.toHaveBeenCalled();
  });
});
