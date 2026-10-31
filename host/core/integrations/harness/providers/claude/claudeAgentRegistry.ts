/**
 * Claude agent runs outlive the turn that dispatched them. A sub-agent resumed
 * with `SendMessage` reports its work under the original Agent call's id, so
 * the session must remember which call dispatched which agent, and which block
 * is currently showing that agent's resumed work.
 *
 * Port of Gespan Native's `crates/gespan-core/src/provider/agent_registry.rs`;
 * both apps must route the same frames to the same blocks.
 */
import type { Block } from "../../../../features/sessions/model/session";

/** Synthetic block ids for work whose parent this session cannot place. */
export const FALLBACK_PREFIX = "background:";
/** Synthetic block ids for stray work of a known but finished dispatch. */
export const CONTINUED_PREFIX = "continued:";

type Dispatch = { agentId?: string; name: string };

export class AgentRegistry {
  /** Agent tool_use id -> what it dispatched. */
  private dispatches = new Map<string, Dispatch>();
  /** Agent id -> the Agent tool_use id that dispatched it. */
  private agents = new Map<string, string>();
  /** Resumption or continuation block id -> the agent id it shows. */
  private resumes = new Map<string, string>();
  /** Agent id -> the block receiving its live resumed work. */
  private active = new Map<string, string>();

  noteDispatch(toolId: string, agentId: string | undefined, name: string): void {
    if (this.resumes.has(toolId)) return;
    let dispatch = this.dispatches.get(toolId);
    if (!dispatch) {
      dispatch = { name: "" };
      this.dispatches.set(toolId, dispatch);
    }
    if (name.trim()) dispatch.name = name.trim();
    if (agentId) this.noteAgentId(toolId, agentId);
  }

  noteAgentId(toolId: string, agentId: string): void {
    if (this.resumes.has(toolId) || !agentId.trim()) return;
    let dispatch = this.dispatches.get(toolId);
    if (!dispatch) {
      dispatch = { name: "" };
      this.dispatches.set(toolId, dispatch);
    }
    dispatch.agentId = agentId;
    this.agents.set(agentId, toolId);
  }

  /** `sendToolId` now shows `agentId`'s resumed work; the latest resume wins. */
  noteResume(sendToolId: string, agentId: string): void {
    this.dispatches.delete(sendToolId);
    this.resumes.set(sendToolId, agentId);
    this.active.set(agentId, sendToolId);
  }

  /** The agent's resumed run ended; later stray frames get a continuation. */
  noteFinished(agentId: string): void {
    this.active.delete(agentId);
  }

  /**
   * A new process cannot continue an old process's resumed streams. The
   * dispatches stay; nothing is active any more.
   */
  deactivateAll(): void {
    this.active.clear();
  }

  /** Routing rule 2: the active resumption block of the agent `parent` dispatched. */
  targetForParent(parent: string): string | undefined {
    const agent = this.dispatches.get(parent)?.agentId;
    return agent ? this.active.get(agent) : undefined;
  }

  /** The active resumption block for an agent id reported on a child frame. */
  targetForAgent(agentId: string): string | undefined {
    return this.active.get(agentId);
  }

  /**
   * Task events are keyed to the call that started the task (an Agent or a
   * resuming SendMessage), and otherwise to the agent's task id.
   */
  targetForTask(
    toolUseId: string | undefined,
    taskId: string | undefined,
  ): string | undefined {
    if (
      toolUseId &&
      (this.resumes.has(toolUseId) || this.dispatches.has(toolUseId))
    ) {
      return toolUseId;
    }
    if (!taskId) return undefined;
    return this.active.get(taskId) ?? this.agents.get(taskId);
  }

  isResume(toolId: string): boolean {
    return this.resumes.has(toolId);
  }

  resumedAgent(toolId: string): string | undefined {
    return this.resumes.get(toolId);
  }

  /** The dispatch's display name, when `toolId` is a known Agent call. */
  dispatchName(toolId: string): string | undefined {
    return this.dispatches.get(toolId)?.name;
  }

  dispatchAgent(toolId: string): string | undefined {
    return this.dispatches.get(toolId)?.agentId;
  }

  /** The Agent call that dispatched `agentId`, with its name. */
  dispatchOf(agentId: string): { toolId: string; name: string } | undefined {
    const toolId = this.agents.get(agentId);
    if (!toolId) return undefined;
    const dispatch = this.dispatches.get(toolId);
    return dispatch ? { toolId, name: dispatch.name } : undefined;
  }

  /**
   * Rebuild from a conversation's persisted agent-run blocks, as written by
   * either app. A new process cannot continue an old resumption's stream, so
   * nothing is marked active.
   */
  seedFromBlocks(blocks: readonly Block[]): void {
    for (const block of blocks) {
      const id = block.tool?.callId;
      const run = block.agentRun as
        | (Block["agentRun"] & { nativeIds?: unknown })
        | undefined;
      if (block.role !== "tool" || !id || !run) continue;
      if (id.startsWith(FALLBACK_PREFIX)) continue;
      const nativeIds = Array.isArray(run.nativeIds) ? run.nativeIds : [];
      const first = nativeIds.find(
        (value): value is string => typeof value === "string" && !!value,
      );
      const agent = run.agentId || first;
      if (run.resumeOf) {
        if (agent) {
          this.resumes.set(id, agent);
          if (!this.dispatchAgent(run.resumeOf)) {
            this.noteAgentId(run.resumeOf, agent);
          }
        }
        continue;
      }
      this.noteDispatch(id, agent, run.name || block.tool?.title || "");
    }
  }
}
