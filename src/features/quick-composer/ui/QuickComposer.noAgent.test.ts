// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { QuickComposer } from "./QuickComposer";
import { loadLastModelChoice } from "../../sessions/model/models";
import {
  NO_CODING_AGENT_HINT,
  OPEN_AGENT_CLIS_REQUEST_EVENT,
} from "../../sessions/model/noCodingAgent";

const native = vi.hoisted(() => ({
  catalog: (_event: { payload: unknown }) => {},
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn().mockResolvedValue(undefined),
  listen: vi.fn(async (name, callback) => {
    if (name === "quick_composer_catalog") native.catalog = callback;
    return () => {};
  }),
}));
vi.mock("./useQuickPickerMotion", () => ({ useQuickPickerMotion: () => {} }));
vi.mock("./QuickWorkspaceControls", () => ({
  QuickWorkspaceControls: () => null,
}));
vi.mock("./QuickProjectIcon", () => ({
  QuickProjectIcon: () => null,
  loadQuickProjectAppearance: () => ({}),
}));
// The selector is covered on its own; here it just hands over a pick.
vi.mock("./QuickModelSelector", async () => {
  const { createElement } = await import("react");
  return {
    QuickModelSelector: ({
      onChange,
    }: {
      onChange: (model: { harness: string; id: string }) => void;
    }) =>
      createElement(
        "button",
        {
          type: "button",
          "data-pick-model": "",
          onClick: () => onChange({ harness: "claude", id: "claude:opus-5" }),
        },
        "pick",
      ),
  };
});
vi.mock("./useQuickAttachments", () => ({
  useQuickAttachments: () => ({ files: [], clear: () => {} }),
}));
vi.mock("../model/quickComposer", async (actual) => ({
  ...(await actual<object>()),
  loadQuickProjects: () => ["/tmp/project"],
  initialQuickChoice: () => ({ harness: "codex", model: "test" }),
  resolveQuickModel: (choice: { harness: string; model: string }) => ({
    harness: choice.harness,
    id: choice.model,
    name: "Test model",
  }),
}));

let root: Root;
let container: HTMLDivElement;
let prompt: HTMLTextAreaElement;

function input(text: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(prompt, text);
    prompt.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
const button = (label: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) => item.textContent?.trim() === label,
  );
const modelButton = () =>
  [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) =>
    item.textContent?.includes("Test model"),
  )!;
async function catalog(availableHarnesses: string[]) {
  await act(async () =>
    native.catalog({ payload: { models: {}, availableHarnesses } }),
  );
}

beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("requestAnimationFrame", (callback: () => void) => {
    callback();
    return 0;
  });
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
  });
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () =>
    root.render(createElement(QuickComposer, { onShown: () => {} })),
  );
  prompt = container.querySelector("textarea")!;
  input("Ship it");
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

it("keeps the picker enabled before detection finishes", () => {
  expect(modelButton().disabled).toBe(false);
  expect(button("Start")!.disabled).toBe(false);
  expect(button("Open Settings")).toBeUndefined();
});

it("disables the picker and send with the settings hint when no agent is detected", async () => {
  await catalog([]);
  expect(modelButton().disabled).toBe(true);
  expect(modelButton().title).toBe(NO_CODING_AGENT_HINT);
  expect(button("Start")!.disabled).toBe(true);
  expect(button("Start")!.title).toBe(NO_CODING_AGENT_HINT);

  await act(async () =>
    prompt.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    ),
  );
  expect(invoke).not.toHaveBeenCalledWith(
    "quick_composer_submit",
    expect.anything(),
  );

  const settings = button("Open Settings")!;
  expect(settings.title).toBe(NO_CODING_AGENT_HINT);
  await act(async () => settings.click());
  expect(emit).toHaveBeenCalledWith(OPEN_AGENT_CLIS_REQUEST_EVENT);
  expect(invoke).toHaveBeenCalledWith("quick_composer_dismiss");

  // Detection finds a CLI: everything re-enables.
  await catalog(["codex"]);
  expect(modelButton().disabled).toBe(false);
  expect(button("Start")!.disabled).toBe(false);
  expect(button("Open Settings")).toBeUndefined();
});

it("picking a model in the quick composer saves it as the last choice", async () => {
  await catalog(["codex", "claude"]);
  await act(async () => modelButton().click());
  await act(async () =>
    container.querySelector<HTMLButtonElement>("[data-pick-model]")!.click(),
  );
  expect(loadLastModelChoice()).toEqual({
    harness: "claude",
    model: "claude:opus-5",
  });
});
