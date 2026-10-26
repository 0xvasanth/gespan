import type { LinkedWorkEvent } from "./board";
import type { LinkState } from "./linkStatus";

type PrEvent = Extract<LinkedWorkEvent, "pr_opened" | "pr_merged">;

export type PrNotify = (
  kind: "pr",
  reference: string,
  event: LinkedWorkEvent,
) => Promise<void>;

/** The board event a PR's state reports: open or merged; nothing otherwise. */
export function prEvent(state: LinkState | undefined): PrEvent | null {
  if (state === "open") return "pr_opened";
  if (state === "merged") return "pr_merged";
  return null;
}

/**
 * Turns the PR states a board status refresh read into `pr_opened` /
 * `pr_merged` events for the cards linked to each PR. A PR reports when a
 * refresh sees it change into one of those states (a first sighting counts);
 * a state the refresh could not read changes nothing. The store also keeps
 * each event to once per card, so a restart that sees the PR again is
 * harmless. A failed send goes to `report` and is tried again on the next
 * refresh.
 */
export function createPrEvents(
  notify: PrNotify,
  report: (error: unknown) => void = console.error,
) {
  /** The last event sent (or being sent) per PR URL. */
  const sent = new Map<string, PrEvent>();
  return {
    observe(prs: readonly { reference: string; state?: LinkState }[]) {
      for (const { reference, state } of prs) {
        const event = prEvent(state);
        if (!event || sent.get(reference) === event) continue;
        sent.set(reference, event);
        const failed = (error: unknown) => {
          if (sent.get(reference) === event) sent.delete(reference);
          report(error);
        };
        try {
          void notify("pr", reference, event).catch(failed);
        } catch (error) {
          failed(error);
        }
      }
    },
  };
}

export type PrEvents = ReturnType<typeof createPrEvents>;
