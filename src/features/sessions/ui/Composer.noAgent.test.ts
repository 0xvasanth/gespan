// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HarnessId } from "../model/session";

const probe = vi.hoisted(() => ({
  probed: false,
  installed: [] as string[],
}));
vi.mock(
  "../../../../host/core/integrations/harness/core/availabilityState",
  async (importOriginal) => {
    const original =
      await importOriginal<
        typeof import("../../../integrations/harness/core/availabilityState")
      >();
    return {
      ...original,
      hasProbedHarnessAvailability: () => probe.probed,
      isHarnessAvailable: (id: string) => probe.installed.includes(id),
    };
  },
);
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../source-control/hooks/useProjectBranches", () => ({
  useProjectBranchesState: () => ({
    branches: { current: "main", detached: false, branches: [] },
    settled: true,
  }),
}));

import { Composer } from "./Composer";
import { emitHarnessAvailability } from "../../../integrations/harness/core/availabilityState";
import {
  NO_CODING_AGENT_HINT,
  OPEN_AGENT_CLIS_EVENT,
} from "../model/noCodingAgent";

describe("Composer when no coding agent is detected", () => {
  let container: HTMLDivElement;
  let root: Root;
  const onSubmit = vi.fn();

  beforeEach(() => {
    probe.probed = false;
    probe.installed = [];
    onSubmit.mockReset();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  async function render(remoteSession = false, harness: HarnessId = "claude") {
    await act(async () =>
      root.render(
        createElement(Composer, {
          focused: true,
          harness,
          model: "claude:opus-5",
          runtimeMode: "supervised",
          executionCwd: "/repo",
          hideProjectPicker: true,
          hideBranchPicker: true,
          initialDraft: "fix the bug",
          remoteSession,
          onFocus: () => {},
          onCwdChange: () => {},
          onModelChange: () => {},
          onRuntimeModeChange: () => {},
          onSubmit,
        }),
      ),
    );
  }

  const picker = () =>
    container.querySelector<HTMLButtonElement>("[aria-haspopup]");
  const send = () =>
    container.querySelector<HTMLButtonElement>("button.composer-send");
  const settingsAction = () =>
    container.querySelector<HTMLButtonElement>(
      'button[data-no-coding-agent="settings"]',
    );
  const pressEnter = async () => {
    const field = container.querySelector("textarea")!;
    await act(async () => {
      field.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });
  };

  it("disables the picker and send with the settings hint when no agent is detected", async () => {
    probe.probed = true;
    await render();

    expect(picker()?.disabled).toBe(true);
    expect(picker()?.title).toBe(NO_CODING_AGENT_HINT);
    expect(send()?.disabled).toBe(true);
    expect(send()?.title).toBe(NO_CODING_AGENT_HINT);

    // Enter can't slip past the disabled button either.
    await pressEnter();
    expect(onSubmit).not.toHaveBeenCalled();

    const opened = vi.fn();
    window.addEventListener(OPEN_AGENT_CLIS_EVENT, opened);
    await act(async () => settingsAction()!.click());
    window.removeEventListener(OPEN_AGENT_CLIS_EVENT, opened);
    expect(settingsAction()!.title).toBe(NO_CODING_AGENT_HINT);
    expect(opened).toHaveBeenCalledTimes(1);
  });

  it("keeps the picker enabled before detection finishes", async () => {
    await render();
    expect(picker()?.disabled).toBe(false);
    expect(send()?.disabled).toBe(false);
    expect(settingsAction()).toBeNull();
  });

  it("re-enables everything as soon as a CLI is detected", async () => {
    probe.probed = true;
    await render();
    expect(send()?.disabled).toBe(true);

    probe.installed = ["claude"];
    await act(async () => emitHarnessAvailability());
    expect(picker()?.disabled).toBe(false);
    expect(send()?.disabled).toBe(false);
    expect(settingsAction()).toBeNull();
    await pressEnter();
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("leaves a remote session's composer alone", async () => {
    probe.probed = true;
    await render(true);
    expect(send()?.disabled).toBe(false);
    expect(settingsAction()).toBeNull();
  });
});
