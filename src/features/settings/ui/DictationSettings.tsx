import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { MOD, SHIFT } from "../../../platform/tauri/platform";
import { keybindingShortcutLabel } from "../model/settings";
import {
  formatModelSize,
  isCancelledDownload,
  loadDictationModel,
  loadDictationWordsText,
  saveDictationModel,
  saveDictationWordsText,
} from "../../sessions/dictation/prefs";
import { DICTATE_COMMAND } from "../../sessions/dictation/shortcut";

export type ModelInfo = {
  id: string;
  label: string;
  sizeBytes: number;
  status: "missing" | "partial" | "ready";
  downloaded: number;
};

type DownloadEvent =
  | { modelId: string; downloaded: number; total: number }
  | { modelId: string; done: true }
  | { modelId: string; error: string };

const ATTRIBUTION =
  "Speech recognition uses NVIDIA Parakeet TDT 0.6B (CC-BY-4.0), run with sherpa-onnx (Apache-2.0).";

const button =
  "rounded-md bg-selection px-2.5 py-1 text-[12px] font-medium text-content hover:bg-selection-hover disabled:opacity-40";
const quiet =
  "rounded-md px-2.5 py-1 text-[12px] text-content/50 hover:text-content disabled:opacity-40";

function percentOf(downloaded: number, total: number) {
  return total > 0 ? Math.floor((downloaded / total) * 100) : 0;
}

export function DictationSettings() {
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [chosen, setChosen] = useState(loadDictationModel);
  const [words, setWords] = useState(loadDictationWordsText);
  /** Progress per model id for downloads started here or elsewhere. */
  const [progress, setProgress] = useState<
    Record<string, { downloaded: number; total: number }>
  >({});
  const [error, setError] = useState("");

  const refresh = async () => {
    try {
      const next = await invoke<ModelInfo[]>("dictation_models");
      setModels(Array.isArray(next) ? next : []);
    } catch (reason) {
      setError(String(reason));
    }
  };

  useEffect(() => {
    void refresh();
    const unlisten = listen<DownloadEvent>("dictation-download", (event) => {
      const payload = event.payload;
      if ("downloaded" in payload) {
        setProgress((current) => ({
          ...current,
          [payload.modelId]: {
            downloaded: payload.downloaded,
            total: payload.total,
          },
        }));
        return;
      }
      setProgress((current) => {
        const next = { ...current };
        delete next[payload.modelId];
        return next;
      });
      void refresh();
    }).catch(() => null);
    return () => {
      void unlisten.then((stop) => stop?.());
    };
  }, []);

  const choose = (id: string) => {
    setChosen(id);
    saveDictationModel(id);
  };

  const download = async (id: string) => {
    setError("");
    setProgress((current) => ({
      ...current,
      [id]: current[id] ?? { downloaded: 0, total: 0 },
    }));
    try {
      await invoke("dictation_download", { modelId: id });
    } catch (reason) {
      if (!isCancelledDownload(reason)) {
        setError(
          String(reason) === "already-downloading"
            ? "The speech model is already downloading."
            : String(reason),
        );
      }
    } finally {
      setProgress((current) => {
        const next = { ...current };
        delete next[id];
        return next;
      });
      void refresh();
    }
  };

  const remove = async (model: ModelInfo) => {
    const confirmed = await ask(
      `Delete the ${model.label} speech model? It can be downloaded again later.`,
      { title: "Delete speech model", kind: "warning" },
    ).catch(() => false);
    if (!confirmed) return;
    setError("");
    try {
      await invoke("dictation_delete_model", { modelId: model.id });
    } catch (reason) {
      setError(String(reason));
    }
    void refresh();
  };

  const shortcut =
    keybindingShortcutLabel(DICTATE_COMMAND, `${MOD}${SHIFT}M`) ?? "Off";

  return (
    <div className="flex flex-col gap-8">
      <section data-setting-id="dictation-model" id="setting-dictation-model">
        <div className="pb-2.5">
          <h2 className="text-[13px] font-semibold text-content">
            Speech model
          </h2>
          <p className="mt-1 text-[12px] leading-relaxed text-content/45">
            Downloaded once to this device, and recognition runs here too.
          </p>
        </div>
        <div
          role="radiogroup"
          aria-label="Speech model"
          className="overflow-hidden rounded-xl border border-content/10 bg-content/3"
        >
          {models === null ? (
            <p className="px-4 py-3 text-[12px] text-content/45">Loading…</p>
          ) : null}
          {models?.map((model) => {
            const running = progress[model.id];
            const percent = running
              ? percentOf(running.downloaded, running.total)
              : percentOf(model.downloaded, model.sizeBytes);
            return (
              <div
                key={model.id}
                className="flex items-center gap-3 border-b border-content/5 px-4 py-3 text-[12px] last:border-b-0"
              >
                <button
                  type="button"
                  role="radio"
                  aria-checked={chosen === model.id}
                  aria-label={model.label}
                  onClick={() => choose(model.id)}
                  className="flex min-w-0 flex-1 items-center gap-3 text-left"
                >
                  <span
                    aria-hidden
                    className={`grid size-3.5 shrink-0 place-items-center rounded-full border ${
                      chosen === model.id
                        ? "border-accent"
                        : "border-content/25"
                    }`}
                  >
                    {chosen === model.id ? (
                      <span className="size-1.5 rounded-full bg-accent" />
                    ) : null}
                  </span>
                  <span className="min-w-0">
                    <span className="block text-[13px] font-medium text-content">
                      {model.label}
                    </span>
                    <span className="block text-content/45">
                      {formatModelSize(model.sizeBytes)} ·{" "}
                      {running
                        ? `Downloading… ${percent}%`
                        : model.status === "ready"
                          ? "Downloaded"
                          : model.status === "partial"
                            ? `Partly downloaded (${percent}%)`
                            : "Not downloaded"}
                    </span>
                  </span>
                </button>
                {running ? (
                  <>
                    <span
                      aria-hidden
                      className="h-1 w-20 overflow-hidden rounded-full bg-content/10"
                    >
                      <span
                        className="block h-full rounded-full bg-accent"
                        style={{ width: `${percent}%` }}
                      />
                    </span>
                    <button
                      type="button"
                      className={quiet}
                      onClick={() =>
                        void invoke("dictation_cancel_download", {
                          modelId: model.id,
                        })
                      }
                    >
                      Cancel
                    </button>
                  </>
                ) : model.status === "ready" ? (
                  <button
                    type="button"
                    className={quiet}
                    onClick={() => void remove(model)}
                  >
                    Delete
                  </button>
                ) : (
                  <button
                    type="button"
                    className={button}
                    onClick={() => void download(model.id)}
                  >
                    {model.status === "partial" ? "Resume" : "Download"}
                  </button>
                )}
              </div>
            );
          })}
        </div>
        {error ? (
          <p role="alert" className="mt-2 text-[12px] text-red-400">
            {error}
          </p>
        ) : null}
      </section>

      <section data-setting-id="dictation-words" id="setting-dictation-words">
        <div className="pb-2.5">
          <h2 className="text-[13px] font-semibold text-content">
            Custom words
          </h2>
          <p className="mt-1 text-[12px] leading-relaxed text-content/45">
            One per line. Names and identifiers to spell the way you write them,
            on top of the ones Gespan picks up from the project.
          </p>
        </div>
        <textarea
          aria-label="Custom words"
          value={words}
          rows={5}
          spellCheck={false}
          placeholder={"useInboxUnseen\nrusqlite"}
          onChange={(event) => {
            setWords(event.target.value);
            saveDictationWordsText(event.target.value);
          }}
          className="w-full resize-y rounded-xl border border-content/10 bg-content/3 px-3 py-2 font-mono text-[12px] text-content outline-none placeholder:text-content/30 focus:border-content/25"
        />
      </section>

      <section
        data-setting-id="dictation-shortcut"
        id="setting-dictation-shortcut"
        className="flex items-start gap-6 rounded-xl border border-content/10 bg-content/3 px-4 py-3.5"
      >
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-content">Shortcut</div>
          <p className="mt-1 text-[12px] leading-relaxed text-content/45">
            In the composer, tap to start and tap again to stop, or hold to talk
            and release to stop. Change it under Keybindings.
          </p>
        </div>
        <kbd className="shrink-0 rounded-md border border-content/10 bg-content/5 px-2 py-0.5 font-sans text-[12px] text-content/70">
          {shortcut}
        </kbd>
      </section>

      <p className="text-[11px] leading-relaxed text-content/40">
        {ATTRIBUTION}
      </p>
    </div>
  );
}
