// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HarnessId } from "./session";

/**
 * Availability is module state that only ever moves forward (a probe can't be
 * undone), so every test loads a fresh copy of the model layer.
 */
async function load(installed?: HarnessId[]) {
  vi.resetModules();
  const state = await import(
    "../../../integrations/harness/core/availabilityState"
  );
  if (installed) {
    const next = Object.fromEntries(
      (
        [
          "claude",
          "codex",
          "cursor",
          "grok",
          "opencode",
          "pi",
          "omp",
          "fx",
          "hermes",
          "kiro",
          "antigravity",
          "devin",
        ] as HarnessId[]
      ).map((id) => [id, installed.includes(id)]),
    ) as Record<HarnessId, boolean>;
    state.setHarnessAvailability(next);
    state.markHarnessAvailabilityProbed();
  }
  const models = await import("./models");
  const projects = await import("./projectProviders");
  const session = await import("./session");
  const noAgent = await import("./noCodingAgent");
  return { state, models, projects, session, noAgent };
}

beforeEach(() => {
  localStorage.clear();
});

describe("new session provider and model", () => {
  it("new sessions use project default, then last used, then first installed", async () => {
    const { models, projects } = await load(["codex", "kiro", "opencode"]);
    models.saveLastModelChoice("opencode", "opencode:glm-5");

    // 1. The project's default, when it is installed.
    projects.setProjectDefaultProvider("/repo/a", "kiro", "kiro:auto");
    expect(models.defaultSessionChoice("/repo/a")).toEqual({
      harness: "kiro",
      model: "kiro:auto",
    });

    // 2. An uninstalled project default falls to the last used provider,
    // with that provider's own last model.
    projects.setProjectDefaultProvider("/repo/b", "claude", "claude:opus-5");
    expect(models.defaultSessionChoice("/repo/b")).toEqual({
      harness: "opencode",
      model: "opencode:glm-5",
    });

    // 3. Neither installed: the first installed provider in standard order.
    models.saveLastModelChoice("cursor", "cursor:grok-4.6");
    expect(models.defaultSessionChoice("/repo/b")).toEqual({
      harness: "codex",
      model: models.defaultModelId("codex"),
    });
  });

  it("skips a last used provider the project hides", async () => {
    const { models, projects } = await load(["codex", "opencode"]);
    models.saveLastModelChoice("opencode", "opencode:glm-5");
    projects.setProjectProviderHidden("/repo/a", "opencode", true);
    expect(models.defaultSessionChoice("/repo/a").harness).toBe("codex");
    expect(models.defaultSessionChoice("/repo/b")).toEqual({
      harness: "opencode",
      model: "opencode:glm-5",
    });
  });

  it("first time uses the first installed provider and its first model", async () => {
    // Standard order, not a built-in favourite: Claude comes before Cursor.
    const { models } = await load(["kiro", "cursor", "claude"]);
    expect(models.defaultSessionChoice()).toEqual({
      harness: "claude",
      model: models.defaultModelId("claude"),
    });
    const { models: onlyKiro } = await load(["kiro"]);
    const choice = onlyKiro.defaultSessionChoice();
    expect(choice.harness).toBe("kiro");
    expect(choice.model).toBe(onlyKiro.modelsFor("kiro")[0]?.id);
  });

  it("restores each provider's own last model when the provider changes", async () => {
    const { models } = await load(["claude", "codex"]);
    models.saveLastModelChoice("claude", "claude:opus-5");
    models.saveLastModelChoice("codex", "codex:gpt-5.5");
    expect(models.defaultSessionChoice()).toEqual({
      harness: "codex",
      model: "codex:gpt-5.5",
    });
    expect(models.preferredModelId("claude")).toBe("claude:opus-5");
  });

  it("opening a project uses project default, then last used, then first installed", async () => {
    const { session, projects, models } = await load([
      "claude",
      "codex",
      "opencode",
    ]);
    // The conversation it was opened from does not count as a pick.
    const seed = session.newSession("codex", "/repo/a", "codex:gpt-5.5");
    expect(session.newSessionForProject(seed, "/repo/a").harness).toBe(
      "claude",
    );

    models.saveLastModelChoice("opencode", "opencode:glm-5");
    projects.setProjectDefaultProvider("/repo/a", "kiro", "kiro:auto");
    const opened = session.newSessionForProject(seed, "/repo/a");
    expect(opened.harness).toBe("opencode");
    expect(opened.runtimeMode).toBe(seed.runtimeMode);

    projects.setProjectDefaultProvider("/repo/a", "codex", "codex:gpt-5.5");
    expect(session.newSessionForProject(seed, "/repo/a").harness).toBe(
      "codex",
    );
  });

  it("fresh sessions created before detection move to an installed provider", async () => {
    const { models, session, state } = await load();
    models.saveLastModelChoice("claude", "claude:opus-5");
    const fresh = session.newDefaultSession("/repo/a");
    expect(fresh.harness).toBe("claude");

    state.setHarnessAvailability({
      ...Object.fromEntries(session.HARNESSES.map((id) => [id, false])),
      codex: true,
    } as Record<HarnessId, boolean>);
    state.markHarnessAvailabilityProbed();
    const moved = session.withInstalledDefault(fresh);
    expect(moved.harness).toBe("codex");
    expect(moved.model).toBe(models.defaultModelId("codex"));

    // A conversation that already started keeps its provider.
    const started = {
      ...fresh,
      blocks: [{ id: "u1", role: "user" as const, text: "hi" }],
    } as typeof fresh;
    expect(session.withInstalledDefault(started)).toBe(started);
    // An installed choice is left alone.
    expect(session.withInstalledDefault(moved)).toBe(moved);
  });
});

describe("last used choice", () => {
  it("picking a model in a chat saves it as the last choice", async () => {
    const { models } = await load(["claude", "codex", "cursor"]);
    models.saveLastModelSettings({ effort: "max" });
    const picked = models.resolveModel("cursor", "cursor:grok-4.6");

    models.rememberModelPick(picked, { effort: "high", fast: "true" });

    expect(models.loadLastModelChoice()).toEqual({
      harness: "cursor",
      model: picked.id,
    });
    expect(models.loadRecentModelChoices()[0]).toEqual({
      harness: "cursor",
      model: picked.id,
    });
    // The next new conversation starts on it.
    expect(models.defaultSessionChoice()).toEqual({
      harness: "cursor",
      model: picked.id,
    });
    // Controls the user set earlier win; ones never saved fill in.
    expect(models.loadLastModelSettings()).toEqual({
      effort: "max",
      fast: "true",
    });

    // Switching back restores that provider's own last model.
    models.rememberModelPick(models.resolveModel("claude", "claude:opus-5"));
    expect(models.preferredModelId("cursor")).toBe(picked.id);
  });
});

describe("no coding agent detected", () => {
  it("keeps the picker enabled before detection finishes", async () => {
    const { noAgent } = await load();
    expect(noAgent.noCodingAgentDetected()).toBe(false);
  });

  it("is detected only once a probe finds no CLI, and clears when one appears", async () => {
    const { noAgent, state, session } = await load([]);
    expect(noAgent.noCodingAgentDetected()).toBe(true);
    state.setHarnessAvailability({
      ...Object.fromEntries(session.HARNESSES.map((id) => [id, false])),
      kiro: true,
    } as Record<HarnessId, boolean>);
    expect(noAgent.noCodingAgentDetected()).toBe(false);
  });

  it("rejects a phone send with the hint when no agent is detected, unless remote", async () => {
    const before = await load();
    expect(() => before.noAgent.assertPhoneSendAllowed(false)).not.toThrow();

    const { noAgent, state, session } = await load([]);
    expect(() => noAgent.assertPhoneSendAllowed(false)).toThrow(
      noAgent.NO_CODING_AGENT_HINT,
    );
    // A host session runs on the host's CLIs.
    expect(() => noAgent.assertPhoneSendAllowed(true)).not.toThrow();

    state.setHarnessAvailability({
      ...Object.fromEntries(session.HARNESSES.map((id) => [id, false])),
      codex: true,
    } as Record<HarnessId, boolean>);
    expect(() => noAgent.assertPhoneSendAllowed(false)).not.toThrow();
  });

  it("uses the exact settings hint and opens Settings at Agent CLIs", async () => {
    const { noAgent } = await load([]);
    expect(noAgent.NO_CODING_AGENT_HINT).toBe(
      "No coding agent detected. Set one up in Settings → Agent CLIs.",
    );
    const heard = vi.fn();
    window.addEventListener(noAgent.OPEN_AGENT_CLIS_EVENT, heard);
    noAgent.openAgentClisSettings();
    window.removeEventListener(noAgent.OPEN_AGENT_CLIS_EVENT, heard);
    expect(heard).toHaveBeenCalledTimes(1);
  });
});

describe("agentClisSettingsWindow", () => {
  const win = (label: string, focused = false, visible = true) => ({
    label,
    focused,
    visible,
  });

  it("opens Settings in one workspace window, never the panel itself", async () => {
    const { noAgent } = await load();
    // Several windows hear the request; exactly one may act on it.
    expect(
      noAgent.agentClisSettingsWindow([
        win("quick-composer", true),
        win("workspace-b"),
        win("workspace-a", false, false),
        win("pilot-chat-1"),
      ]),
    ).toBe("workspace-b");
    expect(
      noAgent.agentClisSettingsWindow([win("workspace-b"), win("workspace-c", true)]),
    ).toBe("workspace-c");
    expect(
      noAgent.agentClisSettingsWindow([
        win("workspace-b", false, false),
        win("workspace-a", false, false),
      ]),
    ).toBe("workspace-a");
    expect(noAgent.agentClisSettingsWindow([win("quick-composer-git")])).toBeNull();
  });
});

describe("answerAgentClisSettingsRequest", () => {
  const fake = (label: string, focused: boolean, visible: boolean, calls: string[]) => ({
    label,
    isFocused: async () => focused,
    isVisible: async () => visible,
    show: async () => void calls.push(`${label}:show`),
    unminimize: async () => void calls.push(`${label}:unminimize`),
    setFocus: async () => void calls.push(`${label}:focus`),
  });

  it("shows the elected hidden window before focusing it, and opens Settings there only", async () => {
    const { noAgent } = await load();
    const calls: string[] = [];
    const panel = fake("quick-composer", true, true, calls);
    const docked = fake("workspace-a", false, false, calls);
    const other = fake("workspace-b", false, false, calls);
    const all = [panel, docked, other];
    const open = vi.fn();

    expect(await noAgent.answerAgentClisSettingsRequest(other, all, open)).toBe(false);
    expect(open).not.toHaveBeenCalled();
    expect(await noAgent.answerAgentClisSettingsRequest(docked, all, open)).toBe(true);
    expect(calls).toEqual([
      "workspace-a:show",
      "workspace-a:unminimize",
      "workspace-a:focus",
    ]);
    expect(open).toHaveBeenCalledTimes(1);
  });
});
