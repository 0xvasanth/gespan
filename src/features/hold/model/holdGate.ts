/**
 * Where a held turn may stop: never while a foreground command runs. The same
 * rule as Gespan Native's provider hold gate.
 */
import type { HarnessEvent } from "../../../integrations/harness/core/types";

export type HoldPoint = "interrupted" | "awaitingInput" | "background";
export type HoldAction =
  | { kind: "interrupt" }
  | { kind: "settle"; point: Exclude<HoldPoint, "interrupted"> };

const FINISHED = new Set(["completed", "failed", "cancelled", "canceled", "error"]);

export class HoldGate {
  private requested = false;
  private running = new Set<string>();
  private background = new Set<string>();
  /** The provider answered and is only waiting on background work. */
  private yielded = false;
  private settled: HoldPoint | null = null;
  /** The turn has shown activity, so its prompt reached the agent. */
  private seen = false;

  request(): void {
    this.requested = true;
  }

  observe(event: HarnessEvent): void {
    this.seen = true;
    switch (event.type) {
      case "tool.started":
        if (event.status && FINISHED.has(event.status)) return;
        this.running.add(event.callId);
        if (event.background) this.background.add(event.callId);
        else if (!this.background.has(event.callId)) this.woke();
        return;
      case "tool.updated":
        if (event.status && FINISHED.has(event.status)) {
          this.running.delete(event.callId);
        }
        return;
      case "background.updated":
        this.background = new Set(
          (event.details ?? []).flatMap((d) => (d.toolId ? [d.toolId] : [])),
        );
        this.yielded = event.tasks.length > 0;
        return;
      case "message.delta":
      case "reasoning.delta":
        this.woke();
        return;
      default:
        return;
    }
  }

  /** New model output after a quiet rest means the agent is working again. */
  private woke(): void {
    if (this.settled === "background") this.settled = null;
    this.yielded = false;
  }

  decide(awaitingInput: boolean): HoldAction | null {
    if (!this.requested || this.settled === "interrupted") return null;
    let point: Exclude<HoldPoint, "interrupted">;
    if (awaitingInput) {
      point = "awaitingInput";
    } else if (!this.seen) {
      // Stopping a turn that is still starting would drop the user's prompt.
      return null;
    } else if ([...this.running].some((id) => !this.background.has(id))) {
      return null;
    } else if (this.yielded) {
      point = "background";
    } else {
      this.settled = "interrupted";
      return { kind: "interrupt" };
    }
    if (this.settled === point) return null;
    this.settled = point;
    return { kind: "settle", point };
  }
}
