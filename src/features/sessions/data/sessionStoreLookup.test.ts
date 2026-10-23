import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { listSessionsByIds } from "./sessionStore";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

beforeEach(() => {
  vi.mocked(invoke).mockReset().mockResolvedValue([]);
});

describe("listSessionsByIds", () => {
  it("asks the store for the given sessions once each", async () => {
    vi.mocked(invoke).mockResolvedValue([
      { id: "s1", cwd: "/work/a", title: "First", archived: true },
    ]);
    const rows = await listSessionsByIds(["s1", "s2", "s1"]);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("session_list_by_ids", {
      sessionIds: ["s1", "s2"],
    });
    expect(rows.map((row) => [row.id, row.archived])).toEqual([["s1", true]]);
  });

  it("skips the call with nothing to look up", async () => {
    expect(await listSessionsByIds([])).toEqual([]);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("splits a long list into calls the store accepts", async () => {
    const ids = Array.from({ length: 1201 }, (_, i) => `s${i}`);
    await listSessionsByIds(ids);
    const sizes = vi
      .mocked(invoke)
      .mock.calls.map(([, args]) => (args as { sessionIds: string[] }).sessionIds.length);
    expect(sizes).toEqual([500, 500, 201]);
  });
});
