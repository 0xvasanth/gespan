import { useCallback, useEffect, useRef, useState } from "react";
import type { HoldState } from "./hold";
import { HoldCoordinator, type HoldHost } from "./holdCoordinator";

export type HoldActions = Omit<HoldHost, "confirm" | "changed">;

/**
 * One HoldCoordinator for the app. `actions` may change every render; the
 * coordinator always calls the latest ones.
 */
export function useHold(actions: HoldActions) {
  const actionsRef = useRef(actions);
  actionsRef.current = actions;
  const resolver = useRef<((ok: boolean) => void) | null>(null);
  const [confirming, setConfirming] = useState(false);
  const coordinatorRef = useRef<HoldCoordinator | null>(null);
  const changedRef = useRef<(state: HoldState | null) => void>(() => undefined);
  if (!coordinatorRef.current) {
    coordinatorRef.current = new HoldCoordinator({
      sessions: () => actionsRef.current.sessions(),
      holdTurn: (id) => actionsRef.current.holdTurn(id),
      stop: (id) => actionsRef.current.stop?.(id),
      resume: (id, prompt) => actionsRef.current.resume(id, prompt),
      confirm: () =>
        new Promise<boolean>((resolve) => {
          resolver.current = resolve;
          setConfirming(true);
        }),
      changed: (state) => changedRef.current(state),
    });
  }
  const hold = coordinatorRef.current;
  const [, setVersion] = useState(0);
  useEffect(() => {
    changedRef.current = () => setVersion((v) => v + 1);
    return () => {
      changedRef.current = () => undefined;
    };
  }, []);
  const answer = useCallback((ok: boolean) => {
    setConfirming(false);
    resolver.current?.(ok);
    resolver.current = null;
  }, []);
  return { hold, phase: hold.phase(), confirming, answer };
}
