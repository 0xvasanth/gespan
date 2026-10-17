import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import { CircleAlert } from "../../../shared/ui/icons";
import { Modal } from "../../../shared/ui/Modal";
import {
  sessionDisplayTitle,
  type Session,
} from "../../sessions/model/session";
import {
  emptyPressure,
  samplePressure,
  type PressureState,
  type ResourceSample,
} from "../model/resourcePressure";

type OwnedProcess = {
  pid: number;
  identity: string;
  sessionId: string;
  cwd: string;
  command: string;
  cpuPercent: number;
  memoryBytes: number;
  agent: boolean;
};
type Props = {
  sessions: RefObject<Session[]>;
  onStopAgent: (id: string) => void;
  onStopSession: (id: string) => void;
  onPressureChange?: (active: boolean) => void;
  /** The footer's slot. Without one, the notice shows as a toast. */
  chipHost?: HTMLElement | null;
};

/** Local subscriptions keep resource sampling out of the workspace render loop. */
export function ResourcePressureNotice({
  sessions,
  onStopAgent,
  onStopSession,
  onPressureChange,
  chipHost,
}: Props) {
  const [pressure, setPressure] = useState<PressureState>(emptyPressure);
  const [sample, setSample] = useState<ResourceSample>();
  const [open, setOpen] = useState(false);
  const [processes, setProcesses] = useState<OwnedProcess[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [stopError, setStopError] = useState("");
  const [stopping, setStopping] = useState<string>();
  const currentOpen = useRef(open);
  currentOpen.current = open;
  useEffect(() => {
    onPressureChange?.(pressure.active);
  }, [pressure.active, onPressureChange]);

  useEffect(() => {
    let alive = true;
    let timer = 0;
    let lastTick = performance.now();
    let stalls = 0;
    let lastPoll = lastTick;
    let state = emptyPressure();
    let polling = false;
    const tick = () => {
      const now = performance.now();
      if (document.hidden) {
        stalls = 0;
        lastPoll = now;
        state = { ...state, high: 0, low: 0 };
      } else {
        const delay = now - lastTick - 500;
        if (delay > 100) stalls += delay;
        if (!polling && now - lastPoll >= 3_000) {
          const stallShare = Math.min(1, stalls / (now - lastPoll));
          stalls = 0;
          lastPoll = now;
          polling = true;
          void invoke<Omit<ResourceSample, "stallShare">>("resource_usage")
            .then((usage) => {
              if (!alive) return;
              const sample = { ...usage, stallShare };
              state = samplePressure(state, sample, now);
              setPressure((previous) =>
                previous.active === state.active &&
                previous.reasons.join() === state.reasons.join()
                  ? previous
                  : state,
              );
              if (currentOpen.current) setSample(sample);
            })
            .catch(() => {})
            .finally(() => {
              polling = false;
            });
        }
      }
      lastTick = now;
      timer = window.setTimeout(tick, 500);
    };
    timer = window.setTimeout(tick, 500);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    let alive = true;
    let timer = 0;
    setLoading(true);
    const refresh = async () => {
      try {
        const rows = await invoke<OwnedProcess[]>("resource_processes");
        if (alive) {
          // Keep controls under the pointer while CPU usage changes between polls.
          setProcesses((previous) => {
            const remaining = new Map(rows.map((row) => [row.identity, row]));
            const ordered: OwnedProcess[] = [];
            for (const old of previous) {
              const row = remaining.get(old.identity);
              if (row) ordered.push(row);
              remaining.delete(old.identity);
            }
            return [...ordered, ...remaining.values()];
          });
          setError("");
        }
      } catch (error) {
        if (alive) setError(String(error));
      } finally {
        if (alive) {
          setLoading(false);
          timer = window.setTimeout(refresh, 3_000);
        }
      }
    };
    void refresh();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [open, stopping]);

  if (!pressure.active && !open) return null;
  const sessionFor = (id: string) =>
    sessions.current.find((session) => session.id === id);
  const titleFor = (id: string) => {
    const session = sessionFor(id);
    return session
      ? sessionDisplayTitle(session.title, session.harness)
      : "Background agent";
  };
  const stopTool = async (process: OwnedProcess) => {
    setStopError("");
    setStopping(process.identity);
    try {
      await invoke("resource_stop_tool", {
        pid: process.pid,
        identity: process.identity,
      });
    } catch (error) {
      setStopError(String(error));
    } finally {
      setStopping(undefined);
    }
  };
  return (
    <>
      {pressure.active && chipHost
        ? createPortal(
            <button
              type="button"
              onClick={() => setOpen(true)}
              title={`${pressure.reasons.join(" · ")} · View processes`}
              className="inline-flex h-5 shrink-0 items-center gap-1 whitespace-nowrap rounded px-1.5 text-content/70 hover:bg-content/10 hover:text-content"
            >
              <CircleAlert
                className="size-3.5 shrink-0 text-amber-400"
                strokeWidth={1.75}
                aria-hidden
              />
              <span>Resource pressure</span>
            </button>,
            chipHost,
          )
        : null}
      {pressure.active && !chipHost ? (
        <article
          className="approval-toast pointer-events-auto overflow-hidden rounded-xl border border-content/20 border-dashed bg-content/10 shadow-xl backdrop-blur-xl"
          role="status"
        >
          <button
            type="button"
            onClick={() => setOpen(true)}
            className="flex w-full items-center gap-2 px-3.5 py-3 text-left hover:bg-content/5"
          >
            <CircleAlert className="size-4 shrink-0 text-amber-400" />
            <span className="min-w-0 flex-1">
              <span className="block text-[13px] font-semibold">
                Resource pressure
              </span>
              <span className="block text-[12px] text-content/60">
                {pressure.reasons.join(" · ")}
              </span>
            </span>
            <span className="text-[11px] text-content/50">View processes</span>
          </button>
        </article>
      ) : null}
      {open ? (
        <Modal
          title="Resource pressure"
          size="lg"
          fitViewport
          onClose={() => setOpen(false)}
        >
          <div className="space-y-4 p-4 text-[13px]">
            <p className="text-content/70">
              {pressure.active
                ? `Sustained ${pressure.reasons.join(" and ").toLowerCase()} was detected.`
                : "Resource pressure has recovered."}{" "}
              Processes below were launched by MonoCode agents. Nothing is
              stopped automatically.
            </p>
            {sample ? (
              <p className="text-content/60">
                CPU:{" "}
                {sample.cpuPercent == null
                  ? "unavailable"
                  : `${Math.round(sample.cpuPercent)}%`}{" "}
                · Memory:{" "}
                {sample.memoryPressure == null
                  ? "unavailable"
                  : sample.memoryPressure >= 4
                    ? "critical"
                    : sample.memoryPressure >= 2
                      ? "under pressure"
                      : "normal"}{" "}
                · UI stall time: {Math.round(sample.stallShare * 100)}%
              </p>
            ) : null}
            <p className="text-content/60">
              Stop build/tool stops that process tree while keeping the
              conversation. Stop agent interrupts its current agent. Stop
              session also stops its orchestration run and pauses queued work.
            </p>
            {error ? (
              <p role="alert" className="text-red-400">
                {error}
              </p>
            ) : null}
            {stopError ? (
              <p role="alert" className="text-red-400">
                {stopError}
              </p>
            ) : null}
            {loading ? (
              <p>Loading processes…</p>
            ) : !processes.length ? (
              <p>
                No running MonoCode-owned processes were found. Other
                applications may be causing the pressure.
              </p>
            ) : null}
            <div className="divide-y divide-stroke">
              {processes.map((process) => (
                <div key={process.identity} className="space-y-1 py-3">
                  <div className="flex items-center gap-3">
                    <span className="min-w-0 flex-1 truncate font-medium">
                      {titleFor(process.sessionId)}
                    </span>
                    <span className="shrink-0 tabular-nums text-content/60">
                      {process.cpuPercent.toFixed(0)}% CPU ·{" "}
                      {(process.memoryBytes / 1024 / 1024).toFixed(0)} MB
                    </span>
                    <button
                      type="button"
                      aria-label={`${process.agent ? "Stop agent" : "Stop build/tool"} for ${titleFor(process.sessionId)} (PID ${process.pid})`}
                      disabled={
                        !!stopping ||
                        (process.agent && !sessionFor(process.sessionId))
                      }
                      className="shrink-0 rounded-md border border-stroke px-2 py-1 hover:bg-content/8 disabled:opacity-40"
                      onClick={() =>
                        process.agent
                          ? onStopAgent(process.sessionId)
                          : void stopTool(process)
                      }
                    >
                      {process.agent ? "Stop agent" : "Stop build/tool"}
                    </button>
                  </div>
                  <code className="block break-all text-[11px] text-content/70">
                    {process.command}
                  </code>
                  <p
                    className="truncate text-[11px] text-content/45"
                    title={process.cwd}
                  >
                    {process.cwd} · PID {process.pid}
                  </p>
                </div>
              ))}
            </div>
            {[...new Set(processes.map((process) => process.sessionId))]
              .filter((id) => sessionFor(id))
              .map((id) => (
                <div
                  key={id}
                  className="flex items-center justify-between gap-3 border-t border-stroke pt-3"
                >
                  <span className="truncate">{titleFor(id)}</span>
                  <button
                    type="button"
                    aria-label={`Stop session ${titleFor(id)}`}
                    className="shrink-0 rounded-md border border-stroke px-3 py-1.5 hover:bg-content/8"
                    onClick={() => onStopSession(id)}
                  >
                    Stop session
                  </button>
                </div>
              ))}
          </div>
        </Modal>
      ) : null}
    </>
  );
}
