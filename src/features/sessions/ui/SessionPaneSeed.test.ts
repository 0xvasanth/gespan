// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SessionPane, type SessionPaneProps } from "./SessionPane";
import { clearComposerDraft } from "../model/draftCache";

type ComposerProbe = {
  initialDraft?: string;
  onDraftChange?: (text: string) => void;
};
const probes = vi.hoisted(() => ({ composer: [] as ComposerProbe[] }));
// The composer as SessionPane drives it: what it is handed, and a way to
// report typed text back.
vi.mock("./Composer", () => ({
  Composer: (props: ComposerProbe) => {
    probes.composer.push(props);
    return null;
  },
}));
vi.mock("../hooks/useFileDrop", () => ({ useFileDrop: () => false }));
vi.mock("./AgentTranscript", () => ({ AgentTranscript: () => null }));

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  vi.stubGlobal(
    "requestAnimationFrame",
    vi.fn(() => 1),
  );
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  localStorage.clear();
  probes.composer = [];
  clearComposerDraft("chat");
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  clearComposerDraft("chat");
  vi.unstubAllGlobals();
});

const noop = () => {};
function props(composerSeed: string): SessionPaneProps {
  return {
    session: {
      id: "chat",
      title: "Chat",
      cwd: "/repo",
      harness: "codex",
      model: "",
      modelSettings: {},
      runtimeMode: "supervised",
      blocks: [],
      composerSeed,
    },
    visible: true,
    focused: true,
    inSplit: false,
    composerFocused: false,
    recents: [],
    onFocus: noop,
    onClose: noop,
    onCwdChange: noop,
    onBranchChange: noop,
    onWorkspaceModeChange: noop,
    onWorktreeBaseChange: noop,
    onModelChange: noop,
    onModelSettingsChange: noop,
    onRuntimeModeChange: noop,
    onSubmit: noop,
    onSaveDraft: noop,
    onRemoveDraft: noop,
    onStop: noop,
    onCompactContext: () => false,
    onPlaceSessionInFolder: noop,
    onDeleteQueuedMessage: noop,
    onEditQueuedMessage: noop,
    onQueuedMessageEditingChange: noop,
    onSteerQueuedMessage: noop,
    onResumeQueue: noop,
    onUsageLimitResume: noop,
    onUsageLimitResumeAtReset: noop,
    onUsageLimitDismiss: noop,
    onApproval: noop,
    onQuestionReply: noop,
    onOpenFile: noop,
    onOpenDiff: noop,
    onOpenPlan: noop,
    onBuildPlan: noop,
    onNewTerminal: noop,
  };
}
function render(pane: SessionPaneProps) {
  act(() => root.render(createElement(SessionPane, pane)));
}
const latest = () => probes.composer[probes.composer.length - 1];

it("session_pane_keeps_edited_text_when_the_composer_seed_changes", () => {
  render(props("ENG-7"));
  expect(latest().initialDraft).toBe("ENG-7");
  // The composer reports its text on mount, then the user edits it.
  act(() => latest().onDraftChange!("ENG-7"));
  act(() => latest().onDraftChange!("ENG-7 check the redirect"));
  render(props("Fix login"));
  expect(latest().initialDraft).toBe("ENG-7 check the redirect");
});

it("session_pane_takes_a_new_composer_seed_while_the_old_one_is_untouched", () => {
  render(props("ENG-7"));
  act(() => latest().onDraftChange!("ENG-7"));
  render(props("Fix login"));
  expect(latest().initialDraft).toBe("Fix login");
});
