// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { DictationSettings, type ModelInfo } from "./DictationSettings";
import {
  DICTATION_MODEL_EN,
  DICTATION_MODEL_MULTI,
  loadDictationModel,
  loadDictationWords,
} from "../../sessions/dictation/prefs";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(async () => true) }));
const events = vi.hoisted(() => ({
  listeners: new Set<(event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (_name: string, handler: (event: { payload: unknown }) => void) => {
      events.listeners.add(handler);
      return () => events.listeners.delete(handler);
    },
  ),
}));

const invokeMock = vi.mocked(invoke);
let container: HTMLDivElement;
let root: Root;
let models: ModelInfo[];

function button(text: string) {
  return [...container.querySelectorAll("button")].find(
    (item) => item.textContent === text,
  )!;
}

async function render() {
  await act(async () => root.render(createElement(DictationSettings)));
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.clear();
  events.listeners.clear();
  models = [
    {
      id: DICTATION_MODEL_EN,
      label: "English (Parakeet v2)",
      sizeBytes: 661_000_000,
      status: "ready",
      downloaded: 661_000_000,
    },
    {
      id: DICTATION_MODEL_MULTI,
      label: "Multilingual (Parakeet v3)",
      sizeBytes: 670_000_000,
      status: "partial",
      downloaded: 335_000_000,
    },
  ];
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command) =>
    command === "dictation_models" ? models : undefined,
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe("DictationSettings", () => {
  it("lists models with sizes and state, English chosen by default", async () => {
    await render();
    const radios = container.querySelectorAll('[role="radio"]');
    expect(radios).toHaveLength(2);
    expect(radios[0].getAttribute("aria-checked")).toBe("true");
    expect(container.textContent).toContain("661 MB · Downloaded");
    expect(container.textContent).toContain("670 MB · Partly downloaded (50%)");
    expect(button("Resume")).toBeDefined();
    expect(container.textContent).toContain(
      "Speech recognition uses NVIDIA Parakeet TDT 0.6B (CC-BY-4.0), run with sherpa-onnx (Apache-2.0).",
    );
  });

  it("persists the chosen model and the custom words", async () => {
    await render();
    await act(async () =>
      container
        .querySelector<HTMLButtonElement>(
          '[aria-label="Multilingual (Parakeet v3)"]',
        )!
        .click(),
    );
    expect(loadDictationModel()).toBe(DICTATION_MODEL_MULTI);

    const words = container.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="Custom words"]',
    )!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      )!.set!;
      setter.call(words, "useInboxUnseen\n\n  rusqlite  \n");
      words.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(loadDictationWords()).toEqual(["useInboxUnseen", "rusqlite"]);
  });

  it("cancelled_download_is_quiet", async () => {
    let reject: (reason: unknown) => void = () => {};
    invokeMock.mockImplementation(async (command) => {
      if (command === "dictation_models") return models;
      if (command === "dictation_download") {
        return new Promise((_, fail) => (reject = fail));
      }
      return undefined;
    });
    await render();
    await act(async () => button("Resume").click());
    await act(async () => {
      for (const handler of events.listeners) {
        handler({
          payload: {
            modelId: DICTATION_MODEL_MULTI,
            downloaded: 402_000_000,
            total: 670_000_000,
          },
        });
      }
    });
    expect(container.textContent).toContain("Downloading… 60%");

    await act(async () => button("Cancel").click());
    expect(invokeMock).toHaveBeenCalledWith("dictation_cancel_download", {
      modelId: DICTATION_MODEL_MULTI,
    });
    await act(async () => {
      for (const handler of events.listeners) {
        handler({
          payload: {
            modelId: DICTATION_MODEL_MULTI,
            error: "download cancelled",
          },
        });
      }
      reject("download cancelled");
    });
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.textContent).not.toContain("Downloading…");
    expect(button("Resume")).toBeDefined();
  });

  it("a failed download shows its error", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "dictation_models") return models;
      if (command === "dictation_download") throw "network error: offline";
      return undefined;
    });
    await render();
    await act(async () => button("Resume").click());
    expect(container.querySelector('[role="alert"]')?.textContent).toBe(
      "network error: offline",
    );
  });

  it("deletes a downloaded model only after confirming", async () => {
    vi.mocked(ask).mockResolvedValueOnce(false);
    await render();
    await act(async () => button("Delete").click());
    expect(invokeMock).not.toHaveBeenCalledWith(
      "dictation_delete_model",
      expect.anything(),
    );

    await act(async () => button("Delete").click());
    expect(invokeMock).toHaveBeenCalledWith("dictation_delete_model", {
      modelId: DICTATION_MODEL_EN,
    });
  });
});
