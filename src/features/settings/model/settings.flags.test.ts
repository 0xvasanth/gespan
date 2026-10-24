// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as settings from "./settings";
import * as appearance from "./appearance";
import { spyOnStorage } from "../../../test/spyOnStorage";

const platform = vi.hoisted(() => ({ isWindows: true }));
vi.mock("../../../platform/tauri/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../platform/tauri/platform")>()),
  get IS_WIN() {
    return platform.isWindows;
  },
}));

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

beforeEach(() => {
  vi.stubGlobal("localStorage", memoryStorage());
  platform.isWindows = true;
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe.each([
  [
    "rebase.tabAnimationsEnabled",
    settings.loadTabAnimationsEnabled,
    settings.saveTabAnimationsEnabled,
    false,
    undefined,
  ],
  [
    "rebase.composerRunner",
    settings.loadComposerRunner,
    settings.saveComposerRunner,
    true,
    "rebase:composer-runner-change",
  ],
  [
    "rebase.notesEnabled",
    settings.loadNotesEnabled,
    settings.saveNotesEnabled,
    true,
    "rebase:notes-enabled-change",
  ],
  [
    "rebase.boardEnabled",
    settings.loadBoardEnabled,
    settings.saveBoardEnabled,
    true,
    "rebase:board-enabled-change",
  ],
  [
    "rebase.liveAgentsEnabled",
    settings.loadLiveAgentsEnabled,
    settings.saveLiveAgentsEnabled,
    true,
    "rebase:live-agents-enabled-change",
  ],
  [
    "rebase.pilotsEnabled",
    settings.loadPilotsEnabled,
    settings.savePilotsEnabled,
    true,
    "rebase:pilots-enabled-change",
  ],
  [
    "rebase.closeToTray",
    settings.loadCloseToTray,
    settings.saveCloseToTray,
    true,
    undefined,
  ],
  [
    "rebase.gridArcadeEnabled",
    settings.loadGridArcadeEnabled,
    settings.saveGridArcadeEnabled,
    true,
    "rebase:grid-arcade-enabled-change",
  ],
  [
    "rebase.claudeHooks",
    settings.loadClaudeHooks,
    settings.saveClaudeHooks,
    true,
    undefined,
  ],
  [
    "rebase.bodyGlass",
    appearance.loadBodyGlass,
    appearance.saveBodyGlass,
    appearance.BODY_GLASS_DEFAULT,
    undefined,
  ],
  [
    "rebase.projectRailOpen",
    appearance.loadProjectRailOpen,
    appearance.saveProjectRailOpen,
    true,
    undefined,
  ],
  [
    "rebase.sessionSidebarOpen",
    appearance.loadSessionSidebarOpen,
    appearance.saveSessionSidebarOpen,
    true,
    undefined,
  ],
  [
    "rebase.transcriptAnchor",
    appearance.loadTranscriptAnchor,
    appearance.saveTranscriptAnchor,
    true,
    "rebase:transcriptanchorchange",
  ],
] as const)("%s", (key, load, save, fallback, eventName) => {
  it("uses its default when unset or unreadable", () => {
    expect(load()).toBe(fallback);
    spyOnStorage(localStorage, "getItem").mockImplementation(() => {
      throw new Error("storage unavailable");
    });
    expect(load()).toBe(fallback);
  });

  it.each([
    ["1", true],
    ["true", true],
    ["0", false],
    ["false", false],
    ["", false],
    ["TRUE", false],
    [" true ", false],
    ["invalid", false],
  ] as const)("reads %j as %s", (stored, expected) => {
    localStorage.setItem(key, stored);
    expect(load()).toBe(expected);
  });

  it.each([false, true])("persists %s before notifying listeners", (value) => {
    const dispatch = vi.spyOn(window, "dispatchEvent");
    const listener = vi.fn((event: Event) => {
      expect((event as CustomEvent<boolean>).detail).toBe(value);
      expect(load()).toBe(value);
    });
    if (eventName) window.addEventListener(eventName, listener);
    try {
      save(value);
      expect(localStorage.getItem(key)).toBe(value ? "1" : "0");
      expect(localStorage.length).toBe(1);
      expect(load()).toBe(value);
      expect(dispatch).toHaveBeenCalledTimes(eventName ? 1 : 0);
      expect(listener).toHaveBeenCalledTimes(eventName ? 1 : 0);
    } finally {
      if (eventName) window.removeEventListener(eventName, listener);
    }
  });

  it("still emits its change event when writing fails", () => {
    const value = !fallback;
    const dispatch = vi.spyOn(window, "dispatchEvent");
    spyOnStorage(localStorage, "setItem").mockImplementation(() => {
      throw new Error("storage quota exceeded");
    });

    expect(() => save(value)).not.toThrow();
    expect(load()).toBe(fallback);
    expect(dispatch).toHaveBeenCalledTimes(eventName ? 1 : 0);
    if (eventName) {
      expect(dispatch).toHaveBeenCalledWith(
        expect.objectContaining({ type: eventName, detail: value }),
      );
    }
  });

  it("tolerates unavailable storage", () => {
    vi.stubGlobal("localStorage", undefined);
    expect(load()).toBe(fallback);
    expect(() => save(!fallback)).not.toThrow();
  });

  it("can persist without a window", () => {
    vi.stubGlobal("window", undefined);
    save(!fallback);
    expect(load()).toBe(!fallback);
  });
});

describe("board enabled subscription", () => {
  it("is on by default and exports its event name", () => {
    expect(settings.BOARD_ENABLED_DEFAULT).toBe(true);
    expect(settings.BOARD_ENABLED_CHANGE_EVENT).toBe(
      "rebase:board-enabled-change",
    );
    expect(settings.loadBoardEnabled()).toBe(true);
  });

  it("notifies subscribers on change and stops after unsubscribing", () => {
    const onChange = vi.fn();
    const unsubscribe = settings.subscribeBoardEnabled(onChange);

    settings.saveBoardEnabled(true);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(settings.loadBoardEnabled()).toBe(true);

    unsubscribe();
    settings.saveBoardEnabled(false);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(settings.loadBoardEnabled()).toBe(false);
  });

  it("does not react to other settings changing", () => {
    const onChange = vi.fn();
    const unsubscribe = settings.subscribeBoardEnabled(onChange);
    settings.saveNotesEnabled(false);
    unsubscribe();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("subscribes safely without a window", () => {
    vi.stubGlobal("window", undefined);
    const unsubscribe = settings.subscribeBoardEnabled(vi.fn());
    expect(() => unsubscribe()).not.toThrow();
  });
});

it("disables close-to-tray outside Windows without consulting storage", () => {
  platform.isWindows = false;
  settings.saveCloseToTray(true);
  const read = spyOnStorage(localStorage, "getItem");

  expect(settings.loadCloseToTray()).toBe(false);
  expect(read).not.toHaveBeenCalled();
});
