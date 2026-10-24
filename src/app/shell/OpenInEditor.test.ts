// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TitleBar, type Tab } from "./TitleBar";
import {
  LAST_EXTERNAL_EDITOR_KEY,
  REVEAL_LABEL,
  openInEditorFolder,
  openInEditorMenuItems,
} from "./openInEditor";

const EDITORS = [
  { id: "vscode", name: "Visual Studio Code" },
  { id: "zed", name: "Zed" },
  { id: "webstorm", name: "WebStorm" },
];

let editors = EDITORS;
let openError: string | null = null;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (command: string) => {
    if (command === "list_external_editors") return editors;
    if (command === "open_in_external_editor" && openError)
      throw new Error(openError);
  }),
  convertFileSrc: (path: string) => path,
}));
vi.mock("./WindowControls", () => ({ WindowControls: () => null }));

const WORKTREE = "/work/app/.worktrees/feature-login";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.clear();
  editors = EDITORS;
  openError = null;
  vi.mocked(invoke).mockClear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

const tab: Tab = {
  id: "t1",
  project: "app",
  title: "Login",
  more: [],
  sessionCount: 1,
  harnesses: [],
  busyHarnesses: [],
  files: [],
};

async function renderTitleBar(
  props: Partial<Parameters<typeof TitleBar>[0]> = {},
) {
  await act(async () =>
    root.render(
      createElement(TitleBar, {
        tabs: [tab],
        activeId: "t1",
        cwd: "/work/app",
        projectRailOpen: true,
        sessionSidebarOpen: true,
        onToggleSidebar: vi.fn(),
        onSelect: vi.fn(),
        onClose: vi.fn(),
        onCloseMany: vi.fn(),
        onReorder: vi.fn(),
        ...props,
      }),
    ),
  );
}

function cornerButton() {
  return container.querySelector<HTMLButtonElement>("[data-open-in-editor]");
}

function menu() {
  return document.querySelector<HTMLElement>('[role="menu"]');
}

function menuLabels() {
  return [...menu()!.querySelectorAll<HTMLButtonElement>("button")].map(
    (item) => item.textContent,
  );
}

function menuItem(label: string) {
  const item = [...menu()!.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent === label,
  );
  expect(item, label).toBeDefined();
  return item!;
}

async function openMenu() {
  await act(async () => cornerButton()!.click());
  expect(menu()).not.toBeNull();
}

describe("openInEditorFolder", () => {
  it("offers local folders, including worktrees", () => {
    expect(openInEditorFolder("/work/app")).toBe("/work/app");
    expect(openInEditorFolder(WORKTREE)).toBe(WORKTREE);
  });

  it("offers nothing for remote, empty or non-project folders", () => {
    expect(openInEditorFolder("remote://host-1/srv/app")).toBeUndefined();
    expect(openInEditorFolder(undefined)).toBeUndefined();
    expect(openInEditorFolder("")).toBeUndefined();
    expect(openInEditorFolder("~")).toBeUndefined();
    expect(openInEditorFolder("/")).toBeUndefined();
  });
});

describe("openInEditorMenuItems", () => {
  const labels = (items: ReturnType<typeof openInEditorMenuItems>) =>
    items.map((item) => (item.kind === "item" ? item.label : "—"));

  it("puts the last editor used first and checks it", () => {
    const items = openInEditorMenuItems(EDITORS, "webstorm");
    expect(labels(items)).toEqual([
      "WebStorm",
      "Visual Studio Code",
      "Zed",
      "—",
      REVEAL_LABEL,
      "Copy path",
    ]);
    expect(items[0]).toMatchObject({ id: "editor:webstorm", checked: true });
    expect(items[1]).toMatchObject({ id: "editor:vscode", checked: undefined });
  });

  it("keeps the scan's order when the last editor was uninstalled", () => {
    const items = openInEditorMenuItems(EDITORS, "cursor");
    expect(labels(items).slice(0, 3)).toEqual([
      "Visual Studio Code",
      "Zed",
      "WebStorm",
    ]);
    expect(items.some((item) => item.kind === "item" && item.checked)).toBe(
      false,
    );
  });

  it("explains a scan in progress or an empty one, keeping folder actions", () => {
    expect(openInEditorMenuItems(null, null)[0]).toMatchObject({
      label: "Looking for editors…",
      disabled: true,
    });
    const none = openInEditorMenuItems([], "zed");
    expect(labels(none)).toEqual([
      "No supported editors found",
      "—",
      REVEAL_LABEL,
      "Copy path",
    ]);
    expect(none[0]).toMatchObject({ disabled: true });
  });
});

describe("title bar Open in editor", () => {
  it("sits in the classic layout's top-right corner", async () => {
    await renderTitleBar({ openInEditorCwd: WORKTREE });
    expect(
      container.querySelector("[data-title-corner] [data-open-in-editor]"),
    ).not.toBeNull();
    expect(cornerButton()!.getAttribute("aria-label")).toBe(
      "Open feature-login in editor",
    );
  });

  it("sits beside the right sidebar toggle in the new layout", async () => {
    await renderTitleBar({
      inset: true,
      openInEditorCwd: WORKTREE,
      onToggleRightPanel: vi.fn(),
    });
    const corner = container.querySelector("[data-title-corner]")!;
    const buttons = [...corner.querySelectorAll("button")];
    expect(buttons.map((button) => button.getAttribute("aria-label"))).toEqual([
      "Open feature-login in editor",
      expect.stringMatching(/^Toggle right sidebar/),
    ]);
  });

  it("is absent without a local folder or while a Pilot fills the window", async () => {
    await renderTitleBar();
    expect(cornerButton()).toBeNull();
    await renderTitleBar({
      openInEditorCwd: WORKTREE,
      pilot: {
        look: { name: "Scout", mascot: "cat", color: "blue" },
        state: { status: "idle" },
      } as never,
    });
    expect(cornerButton()).toBeNull();
  });

  it("opens the session's worktree in the picked editor and remembers it", async () => {
    await renderTitleBar({ openInEditorCwd: WORKTREE });
    await openMenu();
    expect(menu()!.textContent).toContain(WORKTREE);
    expect(menuLabels().slice(0, 3)).toEqual([
      "Visual Studio Code",
      "Zed",
      "WebStorm",
    ]);

    await act(async () => menuItem("WebStorm").click());
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("open_in_external_editor", {
      editorId: "webstorm",
      cwd: WORKTREE,
    });
    expect(menu()).toBeNull();
    expect(localStorage.getItem(LAST_EXTERNAL_EDITOR_KEY)).toBe("webstorm");

    await openMenu();
    expect(menuLabels()[0]).toBe("WebStorm");
    expect(menuItem("WebStorm").getAttribute("aria-checked")).toBe("true");
  });

  it("keeps the menu open with the reason when the editor cannot open", async () => {
    openError = "Zed is no longer installed.";
    await renderTitleBar({ openInEditorCwd: WORKTREE });
    await openMenu();
    await act(async () => menuItem("Zed").click());
    expect(menu()).not.toBeNull();
    expect(menu()!.querySelector('[role="alert"]')!.textContent).toBe(
      "Zed is no longer installed.",
    );
    expect(localStorage.getItem(LAST_EXTERNAL_EDITOR_KEY)).toBeNull();
  });

  it("rescans on open so an editor installed since launch appears", async () => {
    editors = [EDITORS[0]];
    await renderTitleBar({ openInEditorCwd: WORKTREE });
    editors = EDITORS;
    await openMenu();
    expect(menuLabels()).toContain("WebStorm");
  });

  it("reveals or copies the folder", async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    await renderTitleBar({ openInEditorCwd: WORKTREE });
    await openMenu();
    await act(async () => menuItem(REVEAL_LABEL).click());
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("reveal_path", {
      path: WORKTREE,
    });
    expect(menu()).toBeNull();

    await openMenu();
    await act(async () => menuItem("Copy path").click());
    expect(writeText).toHaveBeenCalledWith(WORKTREE);
    expect(menu()).toBeNull();
  });

  it("closes when its button is pressed again", async () => {
    await renderTitleBar({ openInEditorCwd: WORKTREE });
    await openMenu();
    // A real press renders between pointerdown and click.
    await act(async () => {
      cornerButton()!.dispatchEvent(
        new PointerEvent("pointerdown", { bubbles: true }),
      );
    });
    await act(async () => cornerButton()!.click());
    expect(menu()).toBeNull();
    expect(cornerButton()!.getAttribute("aria-expanded")).toBe("false");
  });

  it("follows the focused session to another folder", async () => {
    await renderTitleBar({ openInEditorCwd: WORKTREE });
    await openMenu();
    await renderTitleBar({ openInEditorCwd: "/work/other" });
    expect(menu()).toBeNull();
    await openMenu();
    await act(async () => menuItem("Zed").click());
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("open_in_external_editor", {
      editorId: "zed",
      cwd: "/work/other",
    });
  });
});
