import {
  startSessionFromTicket,
  type TicketSessionHost,
} from "../model/ticketSession";
import { LinkSessionWorkItemDialog } from "./LinkSessionWorkItemDialog";

/**
 * The ticket link dialog in "new session" mode: takes a ticket key or link
 * (a GitHub link is refused) and starts a session linked to it.
 */
export function NewSessionFromTicketDialog({
  host,
  description,
  onStarted,
  onClose,
}: {
  host: TicketSessionHost;
  /** Shown under the heading, such as the project the session opens in. */
  description: string;
  /** The started session's id, once its ticket fetch has settled. */
  onStarted?: (pending: Promise<string>) => void;
  onClose: () => void;
}) {
  return (
    <LinkSessionWorkItemDialog
      ticketOnly
      heading="New session from ticket"
      sessionTitle={description}
      onSave={(item) => {
        if (!item || item.kind !== "ticket") return;
        onClose();
        const pending = startSessionFromTicket(item.ticket, host);
        onStarted?.(pending);
      }}
      onClose={onClose}
    />
  );
}
