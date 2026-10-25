// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { useDictation, type Dictation } from "./useDictation";
import { DICTATION_DEFAULT_MODEL } from "./prefs";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// Delivers backend events the way Tauri's `emit` does.
const events = vi.hoisted(() => ({
  listeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    (name: string, handler: (event: { payload: unknown }) => void) => {
      const set = events.listeners.get(name) ?? new Set();
      set.add(handler);
      events.listeners.set(name, set);
      return Promise.resolve(() => set.delete(handler));
    },
  ),
}));

const invokeMock = vi.mocked(invoke);
const MIC_DENIED =
  "Gespan can't use the microphone. Allow it in System Settings → Privacy & Security → Microphone.";

let root: Root;
let host: HTMLDivElement;
let textarea: HTMLTextAreaElement;
let dictation: Dictation;

function Harness({ sessionId }: { sessionId?: string }) {
  dictation = useDictation({
    fieldRef: { current: textarea },
    projectPath: "/repo",
    sessionId,
  });
  return null;
}

/** A second composer, for the shared-helper cases. */
let otherField: HTMLTextAreaElement;
let other: Dictation;

function OtherHarness() {
  other = useDictation({
    fieldRef: { current: otherField },
    projectPath: "/other",
    sessionId: "s9",
  });
  return null;
}

async function renderBoth() {
  otherField = document.createElement("textarea");
  document.body.append(otherField);
  await act(async () => {
    root.render([
      createElement(Harness, { key: "a", sessionId: "s1" }),
      createElement(OtherHarness, { key: "b" }),
    ]);
  });
}

async function render(sessionId = "s1") {
  await act(async () => {
    root.render(createElement(Harness, { sessionId }));
  });
}

async function emit(name: string, payload: unknown) {
  await act(async () => {
    for (const handler of events.listeners.get(name) ?? []) {
      handler({ payload });
    }
  });
}

const speech = (payload: unknown) => emit("dictation-event", payload);

async function run(fn: () => void | Promise<unknown>) {
  await act(async () => {
    await fn();
  });
}

function calls(command: string) {
  return invokeMock.mock.calls.filter(([name]) => name === command);
}

beforeEach(() => {
  events.listeners.clear();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  localStorage.clear();
  host = document.createElement("div");
  textarea = document.createElement("textarea");
  document.body.append(host, textarea);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  document.body.replaceChildren();
  vi.useRealTimers();
});

describe("useDictation", () => {
  it("tap_toggles_and_hold_releases_stops", async () => {
    vi.useFakeTimers();
    await render();

    // A tap (399 ms) starts dictation and leaves it running.
    await run(() => dictation.pressStart());
    expect(dictation.state).toBe("starting");
    expect(calls("dictation_start")).toEqual([
      [
        "dictation_start",
        {
          attempt: expect.any(Number),
          projectPath: "/repo",
          modelId: DICTATION_DEFAULT_MODEL,
          userWords: [],
          knownNames: [],
        },
      ],
    ]);
    await speech({ type: "listening" });
    expect(dictation.state).toBe("listening");
    await run(() => vi.advanceTimersByTime(399));
    await run(() => dictation.pressEnd());
    expect(calls("dictation_stop")).toHaveLength(0);
    expect(dictation.state).toBe("listening");

    // A second tap stops it.
    await run(() => dictation.pressStart());
    expect(calls("dictation_stop")).toHaveLength(1);
    expect(dictation.state).toBe("finishing");
    await run(() => dictation.pressEnd());
    expect(calls("dictation_stop")).toHaveLength(1);
    await speech({ type: "final", text: "", capped: false });
    expect(dictation.state).toBe("idle");

    // A hold (401 ms) is push-to-talk: releasing stops it.
    await run(() => dictation.pressStart());
    await speech({ type: "listening" });
    await run(() => vi.advanceTimersByTime(401));
    expect(calls("dictation_stop")).toHaveLength(1);
    await run(() => dictation.pressEnd());
    expect(calls("dictation_stop")).toHaveLength(2);
    expect(dictation.state).toBe("finishing");

    // A stray release with no press in flight does nothing.
    await run(() => dictation.pressEnd());
    expect(calls("dictation_stop")).toHaveLength(2);
  });

  it("partial_then_final_inserts_once", async () => {
    textarea.value = "please";
    textarea.setSelectionRange(6, 6);
    await render();
    await run(() => dictation.toggle());
    await speech({ type: "listening" });
    await speech({ type: "level", rms: 0.2 });
    expect(dictation.level).toBe(0.2);

    await speech({ type: "partial", text: "rename use inbox" });
    expect(dictation.partial).toBe("rename use inbox");
    // Partials are captions only; the draft is untouched.
    expect(textarea.value).toBe("please");

    await run(() => dictation.toggle());
    expect(dictation.state).toBe("finishing");
    await speech({
      type: "final",
      text: "rename useInboxUnseen in main.rs",
      capped: false,
    });
    expect(textarea.value).toBe("please rename useInboxUnseen in main.rs");
    expect(dictation.state).toBe("idle");
    expect(dictation.partial).toBe("");
    expect(dictation.level).toBe(0);

    // A repeated final (or one for nobody) is never inserted again.
    await speech({
      type: "final",
      text: "rename useInboxUnseen in main.rs",
      capped: false,
    });
    expect(textarea.value).toBe("please rename useInboxUnseen in main.rs");
  });

  it("capped_final_inserts_without_a_stop", async () => {
    await render();
    await run(() => dictation.toggle());
    await speech({ type: "listening" });
    await speech({ type: "final", text: "long note", capped: true });
    expect(textarea.value).toBe("long note");
    expect(dictation.state).toBe("idle");
    expect(calls("dictation_stop")).toHaveLength(0);
  });

  it("start_error_returns_to_idle", async () => {
    await render();
    await run(() => dictation.toggle());
    await speech({
      type: "error",
      kind: "microphone_denied",
      message: MIC_DENIED,
    });
    expect(dictation.state).toBe("idle");
    expect(dictation.error).toBe(MIC_DENIED);

    // Trying again clears the message and starts a new attempt.
    await run(() => dictation.toggle());
    expect(dictation.error).toBeUndefined();
    expect(dictation.state).toBe("starting");
    expect(calls("dictation_start")).toHaveLength(2);
  });

  it("unavailable_helper_asks_for_restart", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "dictation_start") throw "speech-unavailable";
    });
    await render();
    await run(() => dictation.toggle());
    expect(dictation.state).toBe("error");
    expect(dictation.error).toBe(
      "Dictation is unavailable. Restart Gespan to try again.",
    );
  });

  it("start_without_model_prompts_download", async () => {
    let downloaded = false;
    let finishDownload: () => void = () => {};
    invokeMock.mockImplementation(async (command) => {
      if (command === "dictation_start" && !downloaded) throw "model-missing";
      if (command === "dictation_download") {
        await new Promise<void>((resolve) => {
          finishDownload = () => {
            downloaded = true;
            resolve();
          };
        });
      }
    });
    await render();
    await run(() => dictation.toggle());
    expect(dictation.state).toBe("needs-model");
    expect(calls("dictation_download")).toHaveLength(0);

    await run(() => {
      void dictation.download();
    });
    expect(calls("dictation_download")).toEqual([
      ["dictation_download", { modelId: DICTATION_DEFAULT_MODEL }],
    ]);
    await emit("dictation-download", {
      modelId: DICTATION_DEFAULT_MODEL,
      downloaded: 330,
      total: 660,
    });
    expect(dictation.downloadProgress).toEqual({ downloaded: 330, total: 660 });
    // Progress for the other model is not this prompt's.
    await emit("dictation-download", {
      modelId: "parakeet-tdt-0.6b-v3-multi",
      downloaded: 1,
      total: 670,
    });
    expect(dictation.downloadProgress).toEqual({ downloaded: 330, total: 660 });

    // Once the model is in, dictation starts without another click.
    await run(() => finishDownload());
    expect(calls("dictation_start")).toHaveLength(2);
    expect(dictation.state).toBe("starting");
    expect(dictation.downloadProgress).toBeNull();
  });

  it("not_now_dismisses_the_prompt", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "dictation_start") throw "model-missing";
    });
    await render();
    await run(() => dictation.toggle());
    expect(dictation.state).toBe("needs-model");
    await run(() => dictation.cancel());
    expect(dictation.state).toBe("idle");
    expect(calls("dictation_download")).toHaveLength(0);
  });

  it("cancelled_download_is_quiet", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "dictation_start") throw "model-missing";
      if (command === "dictation_download") {
        // The backend emits the failure and rejects with the same string.
        for (const handler of events.listeners.get("dictation-download") ??
          []) {
          handler({
            payload: {
              modelId: DICTATION_DEFAULT_MODEL,
              error: "download cancelled",
            },
          });
        }
        throw "download cancelled";
      }
    });
    await render();
    await run(() => dictation.toggle());
    await run(() => dictation.download());
    expect(dictation.state).toBe("needs-model");
    expect(dictation.error).toBeUndefined();
    expect(dictation.downloadProgress).toBeNull();
  });

  it("failed_download_shows_the_error", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "dictation_start") throw "model-missing";
      if (command === "dictation_download") throw "network error: offline";
    });
    await render();
    await run(() => dictation.toggle());
    await run(() => dictation.download());
    expect(dictation.state).toBe("needs-model");
    expect(dictation.error).toBe("network error: offline");
    expect(calls("dictation_start")).toHaveLength(1);
  });

  it("escape_cancel_drops_the_final", async () => {
    await render();
    await run(() => dictation.toggle());
    await speech({ type: "listening" });
    await speech({ type: "partial", text: "never mind" });
    await run(() => dictation.cancel());
    expect(calls("dictation_cancel")).toHaveLength(1);
    expect(dictation.state).toBe("idle");
    expect(dictation.partial).toBe("");
    await speech({ type: "final", text: "never mind", capped: false });
    expect(textarea.value).toBe("");
  });

  it("unmount_cancels_without_insert", async () => {
    await render();
    await run(() => dictation.toggle());
    await speech({ type: "listening" });
    await speech({ type: "partial", text: "half a" });
    await act(async () => root.unmount());
    expect(calls("dictation_cancel")).toHaveLength(1);
    await speech({ type: "final", text: "half a thought", capped: false });
    expect(textarea.value).toBe("");
    // Remount so afterEach has something to unmount.
    root = createRoot(host);
  });

  it("idle_unmount_sends_nothing", async () => {
    await render();
    await act(async () => root.unmount());
    expect(calls("dictation_cancel")).toHaveLength(0);
    root = createRoot(host);
  });

  it("session_change_cancels", async () => {
    await render("s1");
    await run(() => dictation.toggle());
    await speech({ type: "listening" });
    await render("s2");
    expect(calls("dictation_cancel")).toHaveLength(1);
    expect(dictation.state).toBe("idle");
    await speech({ type: "final", text: "for s1", capped: false });
    expect(textarea.value).toBe("");
  });

  it("start_resolving_after_cancel_is_ignored", async () => {
    let resolveStart: () => void = () => {};
    invokeMock.mockImplementation(async (command) => {
      if (command === "dictation_start") {
        await new Promise<void>((resolve) => (resolveStart = resolve));
      }
    });
    await render();
    await run(() => {
      dictation.toggle();
    });
    expect(dictation.state).toBe("starting");
    await run(() => dictation.cancel());
    expect(dictation.state).toBe("idle");
    // The cancel waits behind the Start, so the helper gets it afterwards.
    expect(calls("dictation_cancel")).toHaveLength(0);
    await run(() => resolveStart());
    expect(calls("dictation_cancel")).toHaveLength(1);
    await speech({ type: "listening" });
    expect(dictation.state).toBe("idle");
  });

  it("stale_cancelled_does_not_end_next_attempt", async () => {
    await renderBoth();
    await run(() => dictation.toggle());
    await speech({ type: "listening" });
    expect(dictation.state).toBe("listening");

    // Starting in the other composer cancels the first one.
    await run(() => other.toggle());
    expect(dictation.state).toBe("idle");
    expect(calls("dictation_cancel")).toHaveLength(1);
    expect(calls("dictation_start")).toHaveLength(2);

    // The helper answers that cancel, then starts listening for B.
    await speech({ type: "cancelled" });
    expect(other.state).toBe("starting");
    await speech({ type: "listening" });
    expect(other.state).toBe("listening");
    await speech({ type: "partial", text: "for b" });
    expect(other.partial).toBe("for b");
    await run(() => other.toggle());
    await speech({ type: "final", text: "for b", capped: false });
    expect(otherField.value).toBe("for b");
    expect(textarea.value).toBe("");
    expect(other.state).toBe("idle");
  });

  it("another_window_taking_over_returns_to_idle", async () => {
    await render();
    await run(() => dictation.toggle());
    await speech({ type: "listening" });
    await speech({ type: "partial", text: "half" });
    const { attempt } = calls("dictation_start")[0][1] as { attempt: number };

    // A notice for an earlier attempt is not about this one.
    await emit("dictation-taken-over", attempt - 1);
    expect(dictation.state).toBe("listening");

    await emit("dictation-taken-over", attempt);
    expect(dictation.state).toBe("idle");
    expect(dictation.partial).toBe("");
    expect(dictation.error).toBeUndefined();
    // The backend already cancelled the take for the other window.
    expect(calls("dictation_cancel")).toHaveLength(0);
    await speech({ type: "final", text: "half", capped: false });
    expect(textarea.value).toBe("");

    // Ready to start again, as a new attempt.
    await run(() => dictation.toggle());
    expect(dictation.state).toBe("starting");
    const next = calls("dictation_start")[1][1] as { attempt: number };
    expect(next.attempt).toBeGreaterThan(attempt);
  });

  it("stop_and_cancel_name_their_attempt", async () => {
    await render();
    await run(() => dictation.toggle());
    await speech({ type: "listening" });
    const { attempt } = calls("dictation_start")[0][1] as { attempt: number };
    await run(() => dictation.stop());
    expect(calls("dictation_stop")[0][1]).toEqual({ attempt });
    await speech({ type: "final", text: "done", capped: false });

    await run(() => dictation.toggle());
    const second = calls("dictation_start")[1][1] as { attempt: number };
    await run(() => dictation.cancel());
    expect(calls("dictation_cancel")[0][1]).toEqual({
      attempt: second.attempt,
    });
  });

  it("commands_are_serialised", async () => {
    let finishCancel: () => void = () => {};
    let failCancel = false;
    invokeMock.mockImplementation(async (command) => {
      if (command === "dictation_cancel") {
        await new Promise<void>((resolve) => (finishCancel = resolve));
        if (failCancel) throw "helper gone";
      }
    });
    await renderBoth();
    await run(() => dictation.toggle());
    await speech({ type: "listening" });

    await run(() => other.toggle());
    expect(calls("dictation_cancel")).toHaveLength(1);
    // B's Start waits until A's Cancel has reached the backend.
    expect(calls("dictation_start")).toHaveLength(1);
    await run(() => finishCancel());
    expect(calls("dictation_start")).toHaveLength(2);
    expect(calls("dictation_start")[1][1]).toMatchObject({
      projectPath: "/other",
    });

    // A failed command does not block the ones queued after it.
    failCancel = true;
    await run(() => other.cancel());
    await run(() => dictation.toggle());
    expect(calls("dictation_start")).toHaveLength(2);
    await run(() => finishCancel());
    expect(calls("dictation_start")).toHaveLength(3);
    expect(dictation.state).toBe("starting");
  });
});
