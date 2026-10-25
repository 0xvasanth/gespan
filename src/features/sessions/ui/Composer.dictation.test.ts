// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { IS_MAC } from "../../../platform/tauri/platform";
import { Composer } from "./Composer";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
const events = vi.hoisted(() => ({
  listeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (name: string, handler: (event: { payload: unknown }) => void) => {
      const set = events.listeners.get(name) ?? new Set();
      set.add(handler);
      events.listeners.set(name, set);
      return () => set.delete(handler);
    },
  ),
}));

const invokeMock = vi.mocked(invoke);
/** Dictation commands succeed; everything else fails as it does outside Tauri. */
async function backend(command: string) {
  if (command.startsWith("dictation_")) return undefined;
  throw new Error(`${command} is unavailable in tests`);
}
let container: HTMLDivElement;
let root: Root;
let submit: ReturnType<typeof vi.fn>;

async function speech(payload: unknown) {
  await act(async () => {
    for (const handler of events.listeners.get("dictation-event") ?? []) {
      handler({ payload });
    }
  });
}

function calls(command: string) {
  return invokeMock.mock.calls.filter(([name]) => name === command);
}

function chord(type: "keydown" | "keyup", init: KeyboardEventInit = {}) {
  return new KeyboardEvent(type, {
    key: "M",
    code: "KeyM",
    metaKey: IS_MAC,
    ctrlKey: !IS_MAC,
    shiftKey: true,
    bubbles: true,
    cancelable: true,
    ...init,
  });
}

async function render(props: Record<string, unknown> = {}) {
  submit = vi.fn();
  await act(async () =>
    root.render(
      createElement(Composer, {
        focused: false,
        harness: "codex",
        model: "gpt-5",
        runtimeMode: "supervised",
        executionCwd: "/repo",
        sessionId: "s1",
        initialDraft: "fix",
        hideTopBar: true,
        onFocus: vi.fn(),
        onCwdChange: vi.fn(),
        onModelChange: vi.fn(),
        onRuntimeModeChange: vi.fn(),
        onSubmit: submit,
        ...props,
      }),
    ),
  );
  const textarea = container.querySelector("textarea")!;
  textarea.setSelectionRange(3, 3);
  return textarea;
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  events.listeners.clear();
  invokeMock.mockReset();
  invokeMock.mockImplementation(backend);
  localStorage.clear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("Composer dictation", () => {
  it("holding the shortcut talks until release, then the final is sendable", async () => {
    const now = vi.spyOn(Date, "now").mockReturnValue(1_000);
    const textarea = await render();

    await act(async () => textarea.dispatchEvent(chord("keydown")));
    // Auto-repeat while the keys are held does not restart or stop it.
    await act(async () =>
      textarea.dispatchEvent(chord("keydown", { repeat: true })),
    );
    expect(calls("dictation_start")).toHaveLength(1);
    expect(calls("dictation_start")[0][1]).toMatchObject({
      projectPath: "/repo",
      knownNames: expect.arrayContaining(["Codex", "gpt-5"]),
    });
    await speech({ type: "listening" });
    await speech({ type: "partial", text: "the loader" });
    expect(container.textContent).toContain("Listening…");
    expect(container.textContent).toContain("the loader");

    now.mockReturnValue(1_600);
    await act(async () => textarea.dispatchEvent(chord("keyup")));
    expect(calls("dictation_stop")).toHaveLength(1);
    expect(container.textContent).toContain("Finishing…");

    await speech({ type: "final", text: "the loader", capped: false });
    expect(textarea.value).toBe("fix the loader");
    expect(container.querySelector("[data-dictation-bar]")).toBeNull();

    // The draft mirror saw the insertion, so Send sends it.
    await act(async () =>
      container
        .querySelector<HTMLButtonElement>('button[aria-label="Send"]')!
        .click(),
    );
    expect(submit.mock.calls[0][0]).toBe("fix the loader");
  });

  it("escape cancels dictation and claims the key", async () => {
    const textarea = await render();
    await act(async () =>
      container
        .querySelector<HTMLButtonElement>('button[aria-label^="Dictate"]')!
        .click(),
    );
    expect(calls("dictation_start")).toHaveLength(1);
    await speech({ type: "listening" });
    expect(
      container.querySelector('button[aria-label="Stop dictation"]'),
    ).not.toBeNull();

    const escape = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    await act(async () => textarea.dispatchEvent(escape));
    // A claimed Escape is not also taken as "stop the running turn".
    expect(escape.defaultPrevented).toBe(true);
    expect(calls("dictation_cancel")).toHaveLength(1);
    expect(container.querySelector("[data-dictation-bar]")).toBeNull();

    await speech({ type: "final", text: "dropped", capped: false });
    expect(textarea.value).toBe("fix");

    // With nothing dictating, Escape is left alone.
    const idle = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    await act(async () => textarea.dispatchEvent(idle));
    expect(idle.defaultPrevented).toBe(false);
  });

  it("asks before downloading the model", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "dictation_start") throw "model-missing";
      return backend(command);
    });
    await render();
    await act(async () =>
      container
        .querySelector<HTMLButtonElement>('button[aria-label^="Dictate"]')!
        .click(),
    );
    expect(container.textContent).toContain(
      "Download the speech model (about 660 MB)?",
    );
    expect(calls("dictation_download")).toHaveLength(0);
    const notNow = [...container.querySelectorAll("button")].find(
      (button) => button.textContent === "Not now",
    )!;
    await act(async () => notNow.click());
    expect(container.querySelector("[data-dictation-bar]")).toBeNull();
    expect(calls("dictation_download")).toHaveLength(0);
  });

  it("switching session mid-dictation inserts nothing", async () => {
    const textarea = await render();
    await act(async () => textarea.dispatchEvent(chord("keydown")));
    await act(async () => textarea.dispatchEvent(chord("keyup")));
    await speech({ type: "listening" });
    await render({ sessionId: "s2" });
    expect(calls("dictation_cancel")).toHaveLength(1);
    await speech({ type: "final", text: "meant for s1", capped: false });
    expect(textarea.value).toBe("fix");
  });
});
