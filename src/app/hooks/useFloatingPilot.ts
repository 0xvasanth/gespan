import { useEffect, useRef, useSyncExternalStore } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { IS_MAC } from "../../platform/tauri/platform";
import type { Session } from "../../features/sessions/model/session";
import { findPilot } from "../../features/pilots/model/pilot";
import {
  loadPilotMenuBarIcon,
  subscribePilotMenuBarIcon,
} from "../../features/settings/model/settings";
import {
  deliverFloatingPilotRequest,
  floatingPilotRoster,
  floatingPilotMenuMascots,
  floatingPilotSession,
  FLOATING_PILOT_REQUEST,
  type FloatingPilotHost,
  type FloatingPilotRequest,
} from "../../features/pilots/model/floatingPilot";

type Host = Omit<FloatingPilotHost, "create"> & {
  /** Add a Pilot to the roster and return its id. */
  create(): string;
};

/** The floating webview never boots providers or persists the live transcript. */
export function useFloatingPilot(
  sessions: Session[],
  rosterKey: string,
  enabled: boolean,
  host: Host,
) {
  const current = useRef({ sessions, enabled, host });
  current.current = { sessions, enabled, host };
  const active = useRef(new Set<string>());
  const syncing = useRef(Promise.resolve());
  const refresh = useRef<() => void>(() => {});
  const publish = useRef<() => void>(() => {});

  useEffect(() => {
    if (!IS_MAC || !isTauri()) return;
    let disposed = false;
    let draining = false;
    let again = false;
    let unlisten: (() => void) | undefined;
    const lastSessions = new Map<string, Session>();
    const publishSession = (pilotId: string, fallback?: Session) => {
      const sessionId = findPilot(pilotId)?.sessionId;
      const session =
        current.current.sessions.find((s) => s.id === sessionId) ?? fallback;
      if (!session) return;
      lastSessions.set(pilotId, session);
      void invoke("pilot_chat_publish", {
        pilotId,
        session: floatingPilotSession(session),
      }).catch(console.error);
    };
    publish.current = () => {
      for (const pilotId of active.current) {
        const pilot = findPilot(pilotId);
        if (!pilot || !current.current.enabled) {
          active.current.delete(pilotId);
          lastSessions.delete(pilotId);
          continue;
        }
        const session = current.current.sessions.find(
          (s) => s.id === pilot.sessionId,
        );
        if (session && session !== lastSessions.get(pilotId)) {
          publishSession(pilotId);
        }
      }
    };
    const drain = async () => {
      again = true;
      if (draining || disposed) return;
      draining = true;
      try {
        while (again && !disposed) {
          again = false;
          const requests =
            await invoke<FloatingPilotRequest[]>("pilot_chat_take");
          for (const request of requests) {
            if (disposed) break;
            let error: string | null = null;
            try {
              const session = await deliverFloatingPilotRequest(
                request,
                { ...current.current.host, create },
                () =>
                  disposed
                    ? Promise.resolve(false)
                    : invoke<boolean>("pilot_chat_accept", { id: request.id }),
              );
              if (session) {
                active.current.add(request.pilotId);
                publishSession(request.pilotId, session);
              }
            } catch (reason) {
              error = reason instanceof Error ? reason.message : String(reason);
            }
            await invoke("pilot_chat_reply", { id: request.id, error });
          }
        }
      } finally {
        draining = false;
      }
    };
    const roster = () => {
      const pilots = floatingPilotRoster(
        current.current.enabled,
        current.current.sessions,
      );
      const hosted = pilots.flatMap((pilot) => {
        const session = current.current.sessions.find(
          (s) => s.id === pilot.sessionId,
        );
        return session ? [{ pilotId: pilot.id, busy: !!session.busy }] : [];
      });
      return { pilots, hosted, mascots: floatingPilotMenuMascots(pilots) };
    };
    // The native side only opens Pilots it has heard of. This runs inside a
    // drain, so it must not wait on the sync queue, which waits on the drain.
    const create = async (from: string) => {
      const to = current.current.host.create();
      await invoke("pilot_chat_sync", roster());
      await invoke("pilot_chat_switch", { from, to });
    };
    const sync = () => {
      const args = roster();
      syncing.current = syncing.current
        .catch(() => undefined)
        .then(async () => {
          if (disposed) return;
          await invoke("pilot_chat_sync", args);
          await drain();
          publish.current();
        })
        .catch(console.error);
    };
    void getCurrentWebviewWindow()
      .listen(FLOATING_PILOT_REQUEST, () => {
        void drain().catch(console.error);
      })
      .then((stop) => {
        if (disposed) {
          stop();
          return;
        }
        unlisten = stop;
        refresh.current = sync;
        sync();
      })
      .catch(console.error);
    return () => {
      disposed = true;
      refresh.current = () => {};
      publish.current = () => {};
      unlisten?.();
    };
  }, []);

  const roster = floatingPilotRoster(enabled, sessions);
  const hostedKey = roster
    .map((m) => {
      const session = sessions.find((s) => s.id === m.sessionId);
      return `${m.sessionId}:${!!session?.busy}:${m.status}`;
    })
    .join("|");
  useEffect(() => {
    refresh.current();
  }, [rosterKey, enabled, hostedKey]);

  // The native side remembers this too; syncing here keeps it matching the
  // setting if the two ever drift.
  const menuBarIcon = useSyncExternalStore(
    subscribePilotMenuBarIcon,
    loadPilotMenuBarIcon,
  );
  useEffect(() => {
    if (!IS_MAC || !isTauri()) return;
    void invoke("pilot_menu_bar_set_visible", { visible: menuBarIcon }).catch(
      console.error,
    );
  }, [menuBarIcon]);

  // Follow the same React commits as the main transcript. A polling interval
  // batches streamed lines into visible jumps and delays the send entrance.
  // This also keeps publishing when a hidden owner has no animation frames.
  useEffect(() => {
    publish.current();
  }, [sessions]);
}
