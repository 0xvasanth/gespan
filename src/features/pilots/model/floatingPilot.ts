import type { ApprovalDecision } from "../../../integrations/harness";
import type { Attachment, Session } from "../../sessions/model/session";
import type { UserQuestionReply } from "../../sessions/model/userQuestion";
import { displayAttachments } from "../../sessions/model/attachments";
import { listPilots, pilotLook, pilotState, type PilotStatus } from "./pilot";
import { pixelLayers } from "../../projects/model/pixelMascots";
import {
  NO_CODING_AGENT_HINT,
  noCodingAgentDetected,
} from "../../sessions/model/noCodingAgent";

export const FLOATING_PILOT_CHANGED = "pilot_chat_changed";
export const FLOATING_PILOT_REQUEST = "pilot_chat_request";
export type FloatingPilotEntry = {
  id: string;
  name: string;
  mascot: string;
  color: string;
  sessionId: string | null;
  status: PilotStatus;
};
/**
 * The window never probes CLIs, so the owner marks a session published while
 * no coding agent is detected.
 */
export type FloatingPilotSession = Session & { noCodingAgent?: true };
export type FloatingPilotView = {
  pilots: FloatingPilotEntry[];
  pilotId: string | null;
  session: FloatingPilotSession | null;
  error: string | null;
};
export type FloatingPilotAction =
  | { kind: "open" }
  | { kind: "submit"; text: string; attachments: Attachment[] }
  | { kind: "stop" }
  | { kind: "create" }
  | { kind: "approval"; requestId: number; decision: ApprovalDecision }
  | { kind: "question"; requestId: number; reply: UserQuestionReply }
  | { kind: "questionInteraction"; requestId: number }
  | { kind: "reveal" }
  | { kind: "openFile"; path: string }
  | { kind: "openArtifact"; id: string }
  | { kind: "resume" };
export type FloatingPilotRequest = {
  id: number;
  pilotId: string;
  action: FloatingPilotAction;
};

export function floatingPilotRoster(
  enabled: boolean,
  sessions: readonly Session[] = [],
): FloatingPilotEntry[] {
  return enabled
    ? listPilots().map((pilot) => {
        const session = sessions.find((s) => s.id === pilot.sessionId);
        return {
          id: pilot.id,
          ...pilotLook(pilot),
          sessionId: pilot.sessionId ?? null,
          status: session ? pilotState(session).status : "idle",
        };
      })
    : [];
}

/** Draw the same portraits in AppKit without a second set of mascot assets. */
export function floatingPilotMenuMascots(pilots: FloatingPilotEntry[]) {
  return pilots.map((pilot) => {
    const { body, open } = pixelLayers(pilot.mascot, pilot.color);
    return { pilotId: pilot.id, rects: [...body, ...open] };
  });
}

/** Blob URLs belong to their webview; send attachment bytes/paths instead. */
export function floatingPilotSession(session: Session): FloatingPilotSession {
  return {
    ...session,
    ...(noCodingAgentDetected() ? { noCodingAgent: true as const } : {}),
    blocks: session.blocks.map((block) =>
      block.attachments?.length
        ? { ...block, attachments: floatingPilotAttachments(block.attachments) }
        : block,
    ),
    queuedMessages: session.queuedMessages?.map((message) => ({
      ...message,
      attachments: floatingPilotAttachments(message.attachments),
    })),
  };
}

export function floatingPilotAttachments(
  attachments: Attachment[],
): Attachment[] {
  return displayAttachments(attachments).map(
    ({ previewUrl: _preview, ...file }) => file,
  );
}

export type FloatingPilotHost = {
  open(pilotId: string): Promise<Session | undefined>;
  submit(
    sessionId: string,
    text: string,
    attachments: Attachment[],
  ): boolean | void;
  stop(sessionId: string): void;
  approval(
    sessionId: string,
    requestId: number,
    decision: ApprovalDecision,
  ): void;
  question(
    sessionId: string,
    requestId: number,
    reply: UserQuestionReply,
  ): void;
  questionInteraction(sessionId: string, requestId: number): void;
  reveal(pilotId: string): Promise<void>;
  openFile(path: string): void | Promise<void>;
  openArtifact?(pilotId: string, id: string): void | Promise<void>;
  resume(sessionId: string): void;
  /** Add a Pilot and show it in place of the chat that asked. */
  create?(fromPilotId: string): Promise<void>;
};

/** Preparation may await disk; recheck the receipt before mutating a session. */
export async function deliverFloatingPilotRequest(
  request: FloatingPilotRequest,
  host: FloatingPilotHost,
  accept: () => Promise<boolean>,
): Promise<Session | undefined> {
  const session = await host.open(request.pilotId);
  if (!session) throw new Error("That Pilot is no longer available.");
  if (!(await accept())) return undefined;
  const action = request.action;
  switch (action.kind) {
    case "open":
      break;
    case "submit":
      if (noCodingAgentDetected()) throw new Error(NO_CODING_AGENT_HINT);
      if (host.submit(session.id, action.text, action.attachments) === false)
        throw new Error(
          "The message could not be sent. Try again in a moment.",
        );
      break;
    case "stop":
      host.stop(session.id);
      break;
    case "approval":
      host.approval(session.id, action.requestId, action.decision);
      break;
    case "question":
      host.question(session.id, action.requestId, action.reply);
      break;
    case "questionInteraction":
      host.questionInteraction(session.id, action.requestId);
      break;
    case "reveal":
      await host.reveal(request.pilotId);
      break;
    case "openFile":
      await host.openFile(action.path);
      break;
    case "openArtifact":
      if (!host.openArtifact) throw new Error("Artifacts are unavailable.");
      await host.openArtifact(request.pilotId, action.id);
      break;
    case "resume":
      host.resume(session.id);
      break;
    case "create":
      if (!host.create) throw new Error("New Pilots are unavailable here.");
      await host.create(request.pilotId);
      break;
    default:
      throw new Error("Unknown chat action.");
  }
  return session;
}
