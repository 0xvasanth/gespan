import { useState } from "react";
import { Loader, Pause, Play } from "../../../shared/ui/icons";
import { MOD } from "../../../platform/tauri/platform";
import type { HoldPhase } from "../model/hold";

export type SettlingSession = { id: string; title: string; tool?: string };

const chip =
  "inline-flex h-5 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md px-1.5 text-content/55 hover:bg-content/10 hover:text-content";

/**
 * The far right of the status bar: Hold all, Holding… (no undo until every
 * session has stopped), or On hold with Resume.
 */
export function HoldControl({
  phase,
  summary,
  settling,
  onToggle,
  onStopNow,
}: {
  phase: HoldPhase | null;
  summary: string;
  settling: SettlingSession[];
  onToggle: () => void;
  onStopNow: (sessionId: string) => void;
}) {
  const [details, setDetails] = useState(false);
  if (phase === null) {
    return (
      <button
        type="button"
        className={chip}
        aria-label={`Hold everything (${MOD}⇧H)`}
        title={`Hold everything (${MOD}⇧H)`}
        onClick={onToggle}
      >
        <Pause className="size-3.5" strokeWidth={1.75} aria-hidden />
        <span>{summary}</span>
      </button>
    );
  }
  if (phase === "holding") {
    return (
      <span className="relative">
        <button
          type="button"
          className={chip}
          aria-label="Holding: show what is still finishing"
          aria-expanded={details}
          onClick={() => setDetails((open) => !open)}
        >
          <Loader className="size-3.5 animate-spin" aria-hidden />
          <span>{summary}</span>
        </button>
        {details ? (
          <div
            role="dialog"
            aria-label="Still finishing"
            className="absolute right-0 bottom-7 w-80 rounded-lg border border-content/10 bg-surface p-1 text-[12px] shadow-lg"
          >
            <p className="px-2 py-1.5 text-content/50">
              Still finishing. Resume is available once these stop.
            </p>
            {settling.map((session) => (
              <div key={session.id} className="flex items-center gap-2 px-2 py-1.5">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px]">{session.title}</div>
                  <div className="truncate text-content/50">
                    {session.tool ?? "Finishing its current step"}
                  </div>
                </div>
                <button
                  type="button"
                  className="h-6 shrink-0 rounded-md px-2 text-content/70 hover:bg-content/10"
                  onClick={() => onStopNow(session.id)}
                >
                  Stop now
                </button>
              </div>
            ))}
          </div>
        ) : null}
      </span>
    );
  }
  return (
    <span className="flex items-center gap-1">
      <span className="inline-flex items-center gap-1.5 text-content/55">
        <Pause className="size-3.5" strokeWidth={1.75} aria-hidden />
        <span>{summary}</span>
      </span>
      <button
        type="button"
        className={`${chip} text-content/90`}
        aria-label={`Resume everything (${MOD}⇧H)`}
        title={`Resume everything (${MOD}⇧H)`}
        onClick={onToggle}
      >
        <Play className="size-3.5" strokeWidth={1.75} aria-hidden />
        <span>Resume</span>
      </button>
    </span>
  );
}
