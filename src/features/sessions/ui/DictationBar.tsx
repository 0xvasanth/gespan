import { X } from "../../../shared/ui/icons";
import type { Dictation } from "../dictation/useDictation";

const button =
  "rounded-md bg-selection px-2 py-0.5 text-[12px] font-medium text-content hover:bg-selection-hover disabled:opacity-40";
const quiet =
  "rounded-md px-2 py-0.5 text-[12px] text-content/50 hover:text-content";

/** Whether the bar has anything to show for this dictation state. */
export function dictationBarVisible(dictation: Dictation): boolean {
  return dictation.state !== "idle" || !!dictation.error;
}

/**
 * Sits above the composer field while dictation runs or waits on the speech
 * model: a level meter and live caption, or the download prompt.
 */
export function DictationBar({
  dictation,
  onDismiss,
}: {
  dictation: Dictation;
  /** Returns focus to the field after a button click. */
  onDismiss?: () => void;
}) {
  const { state, partial, level, error, downloadProgress } = dictation;
  const keepFocus = (e: { preventDefault: () => void }) => e.preventDefault();

  if (state === "needs-model") {
    const percent =
      downloadProgress && downloadProgress.total > 0
        ? Math.floor(
            (downloadProgress.downloaded / downloadProgress.total) * 100,
          )
        : 0;
    return (
      <div
        data-dictation-bar
        className="flex flex-wrap items-center gap-2 px-3 pt-2 text-[12px] text-content/70"
      >
        {downloadProgress ? (
          <>
            <span className="min-w-0 flex-1">
              Downloading the speech model… {percent}%
            </span>
            <span
              aria-hidden
              className="h-1 w-24 overflow-hidden rounded-full bg-content/10"
            >
              <span
                className="block h-full rounded-full bg-accent transition-[width]"
                style={{ width: `${percent}%` }}
              />
            </span>
            <button
              type="button"
              className={quiet}
              onMouseDown={keepFocus}
              onClick={() => dictation.cancelDownload()}
            >
              Cancel
            </button>
          </>
        ) : (
          <>
            <span className="min-w-0 flex-1">
              Download the speech model (about 660 MB)?
            </span>
            <button
              type="button"
              className={button}
              onMouseDown={keepFocus}
              onClick={() => void dictation.download()}
            >
              Download
            </button>
            <button
              type="button"
              className={quiet}
              onMouseDown={keepFocus}
              onClick={() => {
                dictation.cancel();
                onDismiss?.();
              }}
            >
              Not now
            </button>
          </>
        )}
        {error ? (
          <p role="alert" className="w-full text-red-400">
            {error}
          </p>
        ) : null}
      </div>
    );
  }

  if (state === "idle" || state === "error") {
    if (!error) return null;
    return (
      <div
        data-dictation-bar
        role="alert"
        className="flex items-start gap-2 px-3 pt-2 text-[12px] text-red-400"
      >
        <span className="min-w-0 flex-1">{error}</span>
        <button
          type="button"
          aria-label="Dismiss"
          title="Dismiss"
          className="shrink-0 text-content/40 hover:text-content"
          onMouseDown={keepFocus}
          onClick={() => {
            dictation.cancel();
            onDismiss?.();
          }}
        >
          <X className="size-3.5" />
        </button>
      </div>
    );
  }

  const status =
    state === "finishing"
      ? "Finishing…"
      : state === "starting"
        ? "Starting…"
        : "Listening…";
  // Speech RMS sits well under 0.3, so scale it up to fill the meter.
  const meter = Math.min(1, Math.sqrt(Math.max(0, level)) * 2);
  return (
    <div
      data-dictation-bar
      aria-live="polite"
      className="flex items-center gap-2 px-3 pt-2 text-[12px]"
    >
      <span
        aria-hidden
        className="relative h-3 w-1 shrink-0 overflow-hidden rounded-full bg-content/10"
      >
        <span
          className="absolute inset-x-0 bottom-0 rounded-full bg-accent transition-[height] duration-75"
          style={{ height: `${Math.round(meter * 100)}%` }}
        />
      </span>
      <span className="shrink-0 text-content/70">{status}</span>
      <span className="min-w-0 flex-1 truncate text-content/40">{partial}</span>
      <span className="shrink-0 text-[11px] text-content/35">
        Esc to cancel
      </span>
    </div>
  );
}
