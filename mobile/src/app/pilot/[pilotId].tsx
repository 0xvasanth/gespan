import { Redirect, useLocalSearchParams } from "expo-router";
import { useMemo } from "react";
import { ConnectionNotice } from "@/features/connection/ConnectionNotice";
import { hasNativeTranscript } from "@/features/connection/nativeLink";

import { PixelMascot } from "@/features/mascots/PixelMascot";
import { Composer } from "@/features/sessions/Composer";
import { EmptySession } from "@/features/sessions/EmptySession";
import { SessionScreen } from "@/features/sessions/SessionScreen";
import { Transcript } from "@/features/sessions/Transcript";
import { NativeTranscript } from "@/features/sessions/NativeTranscript";
import type { TranscriptAgent } from "@/features/sessions/TranscriptTurn";
import {
  pilotConversationId,
  useConversation,
  useConversationTurns,
  useWorkspace,
} from "@/features/workspace/WorkspaceProvider";

// A pilot has one long-lived conversation, so its screen is that conversation.
export default function PilotScreen() {
  const { pilotId } = useLocalSearchParams<{ pilotId: string }>();
  const { pilots, sendMessage, stop } = useWorkspace();
  const pilot = pilots.find((m) => m.id === pilotId);
  const conversationId = pilot ? pilotConversationId(pilot.id) : null;
  const conversation = useConversation(conversationId);
  // Stable, so the memoized turns keep their rows across polls.
  const name = pilot?.name ?? "";
  const mascot = pilot?.mascot ?? "";
  const color = pilot?.color ?? "";
  const agent = useMemo(() => ({ name, mascot, color }), [name, mascot, color]);
  if (!pilot || !conversationId) return <Redirect href="/" />;

  return (
    <SessionScreen
      pill={{
        icon: <PixelMascot name={pilot.mascot} color={pilot.color} status={pilot.status} size={28} />,
        label: pilot.name,
      }}
      composer={
        <Composer
          key={pilot.id}
          context={pilot.name}
          branch=""
          model={pilot.model ?? ""}
          harness={pilot.harness}
          busy={conversation.busy}
          onStop={() => stop(conversationId)}
          onSend={(text, attachments) => sendMessage(conversationId, text, attachments)}
        />
      }
    >
      <ConnectionNotice error={conversation.error} />
      {hasNativeTranscript
        ? <NativeTranscript key={pilot.id} conversationId={conversationId} agent={agent} />
        : <PilotTranscript key={pilot.id} conversationId={conversationId} agent={agent} />}
    </SessionScreen>
  );
}

/** The pilot's turns, read here so a streaming reply re-renders only the transcript. */
function PilotTranscript({
  conversationId,
  agent,
}: {
  conversationId: string;
  agent: TranscriptAgent;
}) {
  const { turns, hasOlder, loadOlder } = useConversationTurns(conversationId);
  return turns.length > 0 ? (
    <Transcript turns={turns} agent={agent} hasOlder={hasOlder} loadOlder={loadOlder} />
  ) : (
    <EmptySession />
  );
}
