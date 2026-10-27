// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { Composer } from "./Composer";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));

it("keeps the textarea's text node when a parent re-renders with the latest draft", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (initialDraft: string) =>
    root.render(
      createElement(Composer, {
        focused: false,
        harness: "codex",
        model: "",
        runtimeMode: "supervised",
        executionCwd: "~",
        initialDraft,
        hideTopBar: true,
        onFocus: vi.fn(),
        onCwdChange: vi.fn(),
        onModelChange: vi.fn(),
        onRuntimeModeChange: vi.fn(),
        onSubmit: vi.fn(),
      }),
    );
  try {
    await act(async () => render("你"));
    const textarea = container.querySelector("textarea")!;
    expect(textarea.defaultValue).toBe("你");
    // A remote session's poll re-renders the pane with the draft typed so
    // far. Rewriting the text node makes WebKit commit the IME composition.
    textarea.value = "你好";
    await act(async () => render("你好"));
    expect(textarea.defaultValue).toBe("你");
    expect(textarea.value).toBe("你好");
  } finally {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  }
});

it("takes a replaced parent draft as its own text, so the next render keeps it", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  // SessionPane hands back whatever the composer last reported.
  let parentDraft = "ENG-7";
  const onDraftChange = vi.fn((text: string) => {
    parentDraft = text;
  });
  const render = () =>
    root.render(
      createElement(Composer, {
        focused: false,
        harness: "codex",
        model: "",
        runtimeMode: "supervised",
        executionCwd: "~",
        initialDraft: parentDraft,
        hideTopBar: true,
        onDraftChange,
        onFocus: vi.fn(),
        onCwdChange: vi.fn(),
        onModelChange: vi.fn(),
        onRuntimeModeChange: vi.fn(),
        onSubmit: vi.fn(),
      }),
    );
  try {
    await act(async () => render());
    const textarea = container.querySelector("textarea")!;
    expect(textarea.value).toBe("ENG-7");
    // A ticket's title arrives and replaces the untouched key.
    parentDraft = "Fix login";
    await act(async () => render());
    expect(textarea.value).toBe("Fix login");
    expect(onDraftChange).toHaveBeenLastCalledWith("Fix login");
    // A later unrelated re-render must not bring the key back.
    await act(async () => render());
    expect(textarea.value).toBe("Fix login");
    expect(parentDraft).toBe("Fix login");
  } finally {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  }
});
