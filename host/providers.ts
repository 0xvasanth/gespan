import * as codex from "./core/integrations/harness/providers/codex/codex";
import * as claude from "./core/integrations/harness/providers/claude/claude";
import * as cursor from "./core/integrations/harness/providers/cursor/cursor";
import * as grok from "./core/integrations/harness/providers/grok/grok";
import * as opencode from "./core/integrations/harness/providers/opencode/opencode";
import * as pi from "./core/integrations/harness/providers/pi/pi";
import * as omp from "./core/integrations/harness/providers/omp/omp";
import * as fx from "./core/integrations/harness/providers/fx/fx";
import * as hermes from "./core/integrations/harness/providers/hermes/hermes";
import * as kiro from "./core/integrations/harness/providers/kiro/kiro";
import * as antigravity from "./core/integrations/harness/providers/antigravity/antigravity";
import * as devin from "./core/integrations/harness/providers/devin/devin";
import type {
  SendTurnInput,
  CompactContextInput,
  ApprovalDecision,
} from "./core/integrations/harness/core/types";
import type { Block } from "./core/features/sessions/model/session";
import type { UserQuestionReply } from "./core/features/sessions/model/userQuestion";
import type { RemoteProvider } from "./core/features/connections/model/protocol";
import type { GeneratedSessionTitle } from "./core/features/sessions/model/sessionTitle";
import { generateCodexSessionTitle } from "./core/integrations/harness/providers/codex/codexTitle";
import { generateClaudeSessionTitle } from "./core/integrations/harness/providers/claude/claudeTitle";
import { generateCodexBranchName } from "./core/integrations/harness/providers/codex/codexGit";
import { generateClaudeBranchName } from "./core/integrations/harness/providers/claude/claudeGit";
import { generateCursorSessionTitle } from "./core/integrations/harness/providers/cursor/cursorTitle";
import { generateGrokSessionTitle } from "./core/integrations/harness/providers/grok/grokTitle";
import { generateOpenCodeSessionTitle } from "./core/integrations/harness/providers/opencode/opencodeTitle";
import {
  generatePiSessionTitle,
  generateOmpSessionTitle,
} from "./core/integrations/harness/providers/pi/piTitle";
import {
  PI_FLAVOR,
  OMP_FLAVOR,
} from "./core/integrations/harness/providers/pi/piFlavor";
import { respondQuestion as respondPiQuestion } from "./core/integrations/harness/providers/pi/piFamily";

export interface HostProvider {
  send(input: SendTurnInput): Promise<void>;
  compact?(input: CompactContextInput): Promise<void>;
  cancel(id: string): Promise<void>;
  stop(id: string): Promise<void>;
  bind(id: string, providerId: string, cwd: string): void;
  /**
   * Seed provider agent-run state from the session's blocks after a bind. The
   * host forgets the provider after every turn, so the transcript it holds is
   * what lets a later turn place work from agents dispatched earlier.
   */
  restoreAgentRuns?(id: string, blocks: Block[]): void;
  approve(id: string, request: number, decision: ApprovalDecision): void;
  answer(id: string, request: number, reply: UserQuestionReply): void;
  generateTitle?(input: {
    sessionId: string;
    cwd: string;
    message: string;
  }): Promise<GeneratedSessionTitle | null>;
  generateBranchName?(cwd: string, message: string): Promise<string | null>;
}

export const hostProviders: Record<RemoteProvider, HostProvider> = {
  codex: {
    send: codex.sendCodexTurn,
    compact: codex.compactCodexContext,
    cancel: codex.cancelCodexTurn,
    stop: codex.forgetCodexSession,
    bind: codex.bindCodexSession,
    approve: codex.respondCodexApproval,
    answer: codex.respondCodexQuestion,
    generateTitle: generateCodexSessionTitle,
    generateBranchName: generateCodexBranchName,
  },
  claude: {
    send: claude.sendClaudeTurn,
    compact: claude.compactClaudeContext,
    cancel: claude.cancelClaudeTurn,
    stop: claude.forgetClaudeSession,
    bind: claude.bindClaudeSession,
    restoreAgentRuns: claude.restoreClaudeAgentRuns,
    approve: claude.respondClaudeApproval,
    answer: claude.respondClaudeQuestion,
    generateTitle: generateClaudeSessionTitle,
    generateBranchName: generateClaudeBranchName,
  },
  cursor: {
    send: cursor.sendCursorTurn,
    cancel: cursor.cancelCursorTurn,
    stop: cursor.forgetCursorSession,
    bind: cursor.bindCursorSession,
    approve: cursor.respondCursorApproval,
    answer: cursor.respondCursorQuestion,
    generateTitle: generateCursorSessionTitle,
  },
  grok: {
    send: grok.sendGrokTurn,
    compact: grok.compactGrokContext,
    cancel: grok.cancelGrokTurn,
    stop: grok.forgetGrokSession,
    bind: grok.bindGrokSession,
    approve: grok.respondGrokApproval,
    answer: grok.respondGrokQuestion,
    generateTitle: generateGrokSessionTitle,
  },
  opencode: {
    send: opencode.sendOpenCodeTurn,
    compact: opencode.compactOpenCodeContext,
    cancel: opencode.cancelOpenCodeTurn,
    stop: opencode.forgetOpenCodeSession,
    bind: opencode.bindOpenCodeSession,
    approve: opencode.respondOpenCodeApproval,
    answer: opencode.respondOpenCodeQuestion,
    generateTitle: generateOpenCodeSessionTitle,
  },
  pi: {
    send: pi.sendPiTurn,
    compact: pi.compactPiContext,
    cancel: pi.cancelPiTurn,
    stop: pi.forgetPiSession,
    bind: pi.bindPiSession,
    approve: pi.respondPiApproval,
    answer: (id, request, reply) =>
      respondPiQuestion(PI_FLAVOR, id, request, reply),
    generateTitle: generatePiSessionTitle,
  },
  omp: {
    send: omp.sendOmpTurn,
    compact: omp.compactOmpContext,
    cancel: omp.cancelOmpTurn,
    stop: omp.forgetOmpSession,
    bind: omp.bindOmpSession,
    approve: omp.respondOmpApproval,
    answer: (id, request, reply) =>
      respondPiQuestion(OMP_FLAVOR, id, request, reply),
    generateTitle: generateOmpSessionTitle,
  },
  fx: {
    send: fx.sendFxTurn,
    cancel: fx.cancelFxTurn,
    stop: fx.forgetFxSession,
    bind: fx.bindFxSession,
    approve: fx.respondFxApproval,
    answer: unsupportedQuestion,
  },
  hermes: {
    send: hermes.sendHermesTurn,
    cancel: hermes.cancelHermesTurn,
    stop: hermes.forgetHermesSession,
    bind: hermes.bindHermesSession,
    approve: hermes.respondHermesApproval,
    answer: unsupportedQuestion,
  },
  kiro: {
    send: kiro.sendKiroTurn,
    cancel: kiro.cancelKiroTurn,
    stop: kiro.forgetKiroSession,
    bind: kiro.bindKiroSession,
    approve: kiro.respondKiroApproval,
    answer: unsupportedQuestion,
  },
  antigravity: {
    send: antigravity.sendAntigravityTurn,
    cancel: antigravity.cancelAntigravityTurn,
    stop: antigravity.forgetAntigravitySession,
    bind: antigravity.bindAntigravitySession,
    approve: antigravity.respondAntigravityApproval,
    answer: unsupportedQuestion,
  },
  devin: {
    send: devin.sendDevinTurn,
    compact: devin.compactDevinContext,
    cancel: devin.cancelDevinTurn,
    stop: devin.forgetDevinSession,
    bind: devin.bindDevinSession,
    approve: devin.respondDevinApproval,
    answer: unsupportedQuestion,
    generateTitle: ({ sessionId }) =>
      devin
        .waitForDevinSessionTitle(sessionId)
        .then((title) => (title ? { title, workItem: null } : null)),
  },
};

function unsupportedQuestion(): never {
  throw new Error("This provider does not support questions");
}
