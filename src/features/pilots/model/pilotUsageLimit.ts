import { CONTINUE_PROMPT } from "../../sessions/model/inFlight";
import type { Session } from "../../sessions/model/session";
import { sameProviderAccountId } from "../../providers/model/providerAccounts";
import {
  resumeUsageLimitedSession,
  withUsageLimitAccount,
} from "../../sessions/model/usageLimit";
import { enqueuePilotMessage } from "./pilotMessaging";

/** Retry the existing outbox in order, or continue the stopped work once. */
export function resumePilotUsageLimit(session: Session): Session {
  if (!session.usageLimit || session.busy) return session;
  const resumed = resumeUsageLimitedSession(session);
  return resumed.queuedMessages?.length
    ? resumed
    : enqueuePilotMessage(resumed, {
        id: crypto.randomUUID(),
        text: CONTINUE_PROMPT,
        attachments: [],
      });
}

/** Continue under another account, carrying the native transcript when it
 * was copied there, otherwise through a transcript handoff. */
export function switchPilotUsageLimitAccount(
  session: Session,
  accountId: string,
  carried = false,
): Session {
  if (
    !session.usageLimit ||
    session.busy ||
    sameProviderAccountId(session.providerAccountId, accountId)
  )
    return session;
  return resumePilotUsageLimit(
    withUsageLimitAccount(session, accountId, carried),
  );
}
