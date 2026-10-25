import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState, type RefObject } from "react";
import { insertDictation } from "./insertDictation";
import {
  isCancelledDownload,
  loadDictationModel,
  loadDictationWords,
} from "./prefs";

export type DictationState =
  "idle" | "starting" | "listening" | "finishing" | "needs-model" | "error";

export type DownloadProgress = { downloaded: number; total: number };

export type Dictation = {
  state: DictationState;
  /** Live caption of what has been heard so far; never inserted. */
  partial: string;
  /** Input level (RMS) from the helper, 0 when not listening. */
  level: number;
  error?: string;
  /** Set while this prompt's model download runs. */
  downloadProgress: DownloadProgress | null;
  toggle: () => void;
  /** Stop listening and insert what was heard. */
  stop: () => void;
  pressStart: () => void;
  pressEnd: () => void;
  cancel: () => void;
  download: () => Promise<void>;
  cancelDownload: () => void;
};

type SpeechEvent =
  | { type: "ready" | "loaded" | "listening" | "cancelled" }
  | { type: "level"; rms: number }
  | { type: "partial"; text: string }
  | { type: "final"; text: string; capped: boolean }
  | { type: "error"; kind: string; message: string };

type DownloadEvent =
  | { modelId: string; downloaded: number; total: number }
  | { modelId: string; done: true }
  | { modelId: string; error: string };

/** Outside the Tauri webview there is no event bus; dictation stays idle. */
function subscribe<T>(name: string, handler: (payload: T) => void) {
  return listen<T>(name, (event) => handler(event.payload)).catch(() => null);
}

/** A shortcut press at least this long is push-to-talk: releasing stops. */
export const HOLD_TO_TALK_MS = 400;

const UNAVAILABLE = "Dictation is unavailable. Restart Gespan to try again.";
const ALREADY_DOWNLOADING = "The speech model is already downloading.";

/**
 * The helper is shared, so its events belong to the one composer that last
 * started dictation; starting in another composer cancels this one. Across
 * windows the backend decides: it sends events only to the owning window and
 * tells a window when another one took over.
 */
let owner: { abandon: () => void } | null = null;

/** Numbers this window's attempts, so the backend can tell them apart. */
let attempts = 0;

/**
 * Tauri runs each command on its own thread, so a Cancel for one composer
 * could reach the helper after the next composer's Start. Every speech
 * command waits for the previous one; a failure does not hold up the rest.
 */
let commands: Promise<unknown> = Promise.resolve();

function speechCommand(
  name: "dictation_start" | "dictation_stop" | "dictation_cancel",
  args?: Record<string, unknown>,
): Promise<unknown> {
  const result = commands.then(() => invoke(name, args));
  commands = result.catch(() => undefined);
  return result;
}

/** Fire and forget; the helper reports failures as events. */
function sendSpeech(
  name: "dictation_stop" | "dictation_cancel",
  attempt: number,
) {
  speechCommand(name, { attempt }).catch(() => undefined);
}

type Options = {
  fieldRef: RefObject<HTMLTextAreaElement | null>;
  projectPath?: string;
  sessionId?: string;
  /** Provider and model names to bias identifier joining towards. */
  knownNames?: string[];
};

export function useDictation({
  fieldRef,
  projectPath,
  sessionId,
  knownNames,
}: Options): Dictation {
  const [state, setStateValue] = useState<DictationState>("idle");
  const [partial, setPartial] = useState("");
  const [level, setLevel] = useState(0);
  const [error, setError] = useState<string>();
  const [downloadProgress, setDownloadProgress] =
    useState<DownloadProgress | null>(null);

  const options = useRef({ projectPath, knownNames });
  options.current = { projectPath, knownNames };

  // Everything below runs from events and timers, so it reads refs and is
  // built once.
  const [controller] = useState(() => {
    const ref = {
      state: "idle" as DictationState,
      /** Bumped by every start and cancel; a stale start result is dropped. */
      run: 0,
      /** The backend's name for the latest start; 0 before the first. */
      attempt: 0,
      /** This composer's attempt owns the helper's events. */
      active: false,
      startPending: false,
      stopAfterStart: false,
      press: null as { at: number } | null,
      modelId: loadDictationModel(),
      downloading: null as string | null,
      mounted: true,
    };
    const setState = (next: DictationState) => {
      ref.state = next;
      setStateValue(next);
    };
    const self = { abandon: () => abandon() };

    const release = () => {
      ref.active = false;
      ref.stopAfterStart = false;
      if (owner === self) owner = null;
      setPartial("");
      setLevel(0);
    };

    const start = async () => {
      if (owner && owner !== self) owner.abandon();
      owner = self;
      const run = ++ref.run;
      const attempt = ++attempts;
      ref.attempt = attempt;
      ref.active = true;
      ref.startPending = true;
      ref.stopAfterStart = false;
      ref.modelId = loadDictationModel();
      setError(undefined);
      setPartial("");
      setLevel(0);
      setState("starting");
      try {
        await speechCommand("dictation_start", {
          attempt,
          projectPath: options.current.projectPath ?? null,
          modelId: ref.modelId,
          userWords: loadDictationWords(),
          knownNames: options.current.knownNames ?? [],
        });
      } catch (reason) {
        ref.startPending = false;
        if (run !== ref.run || !ref.active) return;
        release();
        const code = String(reason);
        if (code === "model-missing") {
          setState("needs-model");
        } else if (code === "speech-unavailable") {
          setError(UNAVAILABLE);
          setState("error");
        } else {
          setError(code);
          setState("idle");
        }
        return;
      }
      ref.startPending = false;
      // A cancel sent meanwhile was queued behind this Start, so it already
      // reaches the helper after it.
      if (run !== ref.run) return;
      if (ref.active && ref.stopAfterStart) {
        ref.stopAfterStart = false;
        sendSpeech("dictation_stop", attempt);
      }
    };

    const stop = () => {
      if (!ref.active || ref.state === "finishing") return;
      setState("finishing");
      if (ref.startPending) ref.stopAfterStart = true;
      else sendSpeech("dictation_stop", ref.attempt);
    };

    /** Drop the attempt without inserting anything. */
    const abandon = () => {
      ref.press = null;
      if (ref.active) {
        ref.run++;
        release();
        sendSpeech("dictation_cancel", ref.attempt);
      }
      setError(undefined);
      setState("idle");
    };

    /** Another window started dictation; the backend cancelled this take. */
    const onTakenOver = (attempt: number) => {
      if (!ref.active || attempt !== ref.attempt) return;
      ref.press = null;
      ref.run++;
      release();
      setError(undefined);
      setState("idle");
    };

    const onSpeech = (event: SpeechEvent) => {
      if (owner !== self || !ref.active) return;
      switch (event.type) {
        case "listening":
          if (ref.state === "starting") setState("listening");
          return;
        case "level":
          setLevel(event.rms);
          return;
        case "partial":
          setPartial(event.text);
          return;
        case "final": {
          release();
          const field = fieldRef.current;
          if (field && field.isConnected) insertDictation(field, event.text);
          setState("idle");
          return;
        }
        case "cancelled":
          // Only this hook sends Cancel, and it has already reset by then. A
          // Cancelled seen here answers an earlier attempt's cancel, possibly
          // another composer's, and must not end this one.
          return;
        case "error":
          release();
          if (event.kind === "model_missing") {
            setState("needs-model");
            return;
          }
          setError(event.message);
          setState("idle");
          return;
      }
    };

    const onDownload = (event: DownloadEvent) => {
      if (event.modelId !== ref.downloading || !("downloaded" in event)) return;
      setDownloadProgress({ downloaded: event.downloaded, total: event.total });
    };

    const download = async () => {
      if (ref.downloading) return;
      const modelId = ref.modelId;
      ref.downloading = modelId;
      setError(undefined);
      setDownloadProgress({ downloaded: 0, total: 0 });
      try {
        await invoke("dictation_download", { modelId });
      } catch (reason) {
        ref.downloading = null;
        setDownloadProgress(null);
        if (isCancelledDownload(reason)) return;
        const code = String(reason);
        setError(code === "already-downloading" ? ALREADY_DOWNLOADING : code);
        return;
      }
      ref.downloading = null;
      setDownloadProgress(null);
      if (ref.mounted && ref.state === "needs-model") void start();
    };

    const cancelDownload = () => {
      if (ref.downloading) {
        void invoke("dictation_cancel_download", { modelId: ref.downloading });
      }
    };

    return {
      ref,
      cancelDownload,
      onSpeech,
      onDownload,
      onTakenOver,
      abandon,
      stop,
      download,
      toggle: () => {
        if (ref.state === "idle" || ref.state === "error") void start();
        else if (ref.state === "starting" || ref.state === "listening") stop();
      },
      pressStart: () => {
        if (ref.state === "starting" || ref.state === "listening") {
          ref.press = null;
          stop();
          return;
        }
        if (ref.state !== "idle" && ref.state !== "error") return;
        ref.press = { at: Date.now() };
        void start();
      },
      pressEnd: () => {
        const press = ref.press;
        ref.press = null;
        if (press && Date.now() - press.at >= HOLD_TO_TALK_MS) stop();
      },
    };
  });

  useEffect(() => {
    controller.ref.mounted = true;
    const speech = subscribe<SpeechEvent>("dictation-event", (payload) =>
      controller.onSpeech(payload),
    );
    const download = subscribe<DownloadEvent>("dictation-download", (payload) =>
      controller.onDownload(payload),
    );
    const takenOver = subscribe<number>("dictation-taken-over", (attempt) =>
      controller.onTakenOver(attempt),
    );
    return () => {
      controller.ref.mounted = false;
      void speech.then((unlisten) => unlisten?.());
      void download.then((unlisten) => unlisten?.());
      void takenOver.then((unlisten) => unlisten?.());
    };
  }, [controller]);

  // Leaving the session (or the composer) mid-dictation inserts nothing.
  useEffect(() => {
    return () => controller.abandon();
  }, [controller, sessionId]);

  return {
    state,
    partial,
    level,
    error,
    downloadProgress,
    toggle: controller.toggle,
    stop: controller.stop,
    pressStart: controller.pressStart,
    pressEnd: controller.pressEnd,
    cancel: controller.abandon,
    download: controller.download,
    cancelDownload: controller.cancelDownload,
  };
}
