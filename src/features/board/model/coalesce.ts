/**
 * Coalesces reloads: one run in flight at a time, plus a single trailing run
 * when calls arrived while it was in flight. A burst of `gespan:board-changed`
 * events (a bulk undo restores cards one by one, each a write) then costs at
 * most two reads, and the last one starts after the last event, so what it
 * reads is current. A run that throws doesn't stop the trailing one.
 *
 * The returned promise settles once the run it joined (and any trailing run)
 * has finished.
 */
export function coalesce(run: () => Promise<unknown>): () => Promise<void> {
  let running: Promise<void> | null = null;
  let again = false;
  const loop = async () => {
    try {
      do {
        again = false;
        try {
          await run();
        } catch {
          // The caller reports its own errors; a failed read still re-runs.
        }
      } while (again);
    } finally {
      running = null;
    }
  };
  return () => {
    if (running) {
      again = true;
      return running;
    }
    running = loop();
    return running;
  };
}
