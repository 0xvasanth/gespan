import { UsageLimitCard } from "@/features/sessions/UsageLimitCard";
import {
  Redirect,
  router,
  Stack,
  useIsFocused,
  useLocalSearchParams,
} from "expo-router";
import * as Haptics from "expo-haptics";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Alert, Text } from "react-native";
import { ConnectionNotice } from "@/features/connection/ConnectionNotice";
import { hasNativeTranscript } from "@/features/connection/nativeLink";

import { PixelMascot } from "@/features/mascots/PixelMascot";
import { usePilotModel } from "@/features/models/usePilotModel";
import { useDrawerActions, useDrawerSwipe } from "@/ui/ScenePager";
import { Timed } from "@/ui/Timed";
import { useConnection, usePilotTools } from "@/features/connection/ConnectionProvider";
import { rememberPlace } from "@/features/workspace/lastPlace";
import { markPilotSeen, markPilotUnread } from "@/features/workspace/pilotSeen";
import { Composer } from "@/features/sessions/Composer";
import { PilotChoose, PilotMessageActions } from "@/features/sessions/PilotCards";
import { PilotMenu } from "@/features/sessions/PilotMenu";
import { usePilotAccent } from "@/features/workspace/pilotAccent";
import { AccentOverride, ACCENTS, mix, themedStyles, useAccent, useScheme } from "@/ui/theme";
import { LiquidView } from "@/ui/LiquidView";
import { useNativeHeaderHeight } from "@/ui/useNativeHeaderHeight";
import type { Pilot } from "@/features/workspace/types";
import { QuestionCard } from "@/features/sessions/QuestionCard";
import { EmptySession } from "@/features/sessions/EmptySession";
import {
  HEADER_HEIGHT,
  SessionContent,
  SessionFrame,
  SessionScreen,
} from "@/features/sessions/SessionScreen";
import { usePilotOnlyLayout } from "@/features/settings/pilotOnly";
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
// Pilots only, it is pushed over the list as a session is over its project:
// under the stack's native header, with back on the left and its ⋯ on the
// right as native items, and a frame of its own for its composer, so the
// whole screen slides in and out with the push.
export default function PilotRoute() {
  const [pilotOnly] = usePilotOnlyLayout();
  return pilotOnly ? <PushedPilot /> : <PilotScreen pilotOnly={false} />;
}

function PushedPilot() {
  const headerHeight = useNativeHeaderHeight();
  return (
    <SessionFrame nativeHeaderHeight={headerHeight}>
      <PilotScreen pilotOnly />
    </SessionFrame>
  );
}

function PilotScreen({ pilotOnly }: { pilotOnly: boolean }) {
  const { pilotId } = useLocalSearchParams<{ pilotId: string }>();
  const { pilots, sendMessage, stop } = useWorkspace();
  useDrawerSwipe(!pilotOnly);
  const pilot = pilots.find((m) => m.id === pilotId);
  const conversationId = pilot ? pilotConversationId(pilot.id) : null;
  const conversation = useConversation(conversationId);
  // Where the app opens next time.
  const { command, refresh } = useConnection();
  // The older desktop app has no profile, reset or delete for a Pilot.
  const tools = usePilotTools();
  const desktop = useConnection().pairing?.desktop.fingerprint;
  const placeId = pilot?.id;
  useEffect(() => {
    if (placeId) rememberPlace(desktop, { kind: "pilot", id: placeId });
  }, [desktop, placeId]);
  // Its replies are seen as they arrive while its chat is in front, unless
  // it was just marked unread: that holds until the chat is left.
  const focused = useIsFocused();
  const keptUnread = useRef(false);
  useEffect(() => {
    if (!focused) keptUnread.current = false;
    else if (pilot && !keptUnread.current) markPilotSeen(desktop, pilot);
  }, [focused, desktop, pilot]);
  // A Pilot's message being replied to, quoted over the next one sent.
  const [reply, setReply] = useState<string | null>(null);
  const drawer = useDrawerActions();
  const messageActions = useMemo(
    () => ({
      reply: setReply,
      // As Messages: the chat reads as new again, and the list comes up.
      markUnread: () => {
        if (!pilot) return;
        keptUnread.current = true;
        markPilotUnread(desktop, pilot);
        // Pilots only, the list is the one back.
        if (pilotOnly) router.back();
        else drawer.open();
      },
    }),
    [desktop, pilot, drawer, pilotOnly],
  );
  // Stable, so the memoized turns keep their rows across polls.
  const name = pilot?.name ?? "";
  const mascot = pilot?.mascot ?? "";
  const color = pilot?.color ?? "";
  const agent = useMemo(() => ({ name, mascot, color }), [name, mascot, color]);
  const { model, harness, effort } = usePilotModel(pilot);
  // The colour picked for this Pilot's chat, over the app's accent.
  const pilotAccent = usePilotAccent(pilot?.id);
  const scheme = useScheme();
  // Pilots only, a chat with no colour picked, here or for the app, is drawn
  // in the Pilot's own: as is in the dark, deepened in the light to hold its
  // contrast on the pale background.
  const appAccent = useAccent();
  const ownAccent =
    pilotOnly && appAccent === "theme" && /^#[0-9a-f]{6}$/i.test(color)
      ? scheme === "light"
        ? mix(color, "#000000", 0.22)
        : color
      : null;
  const chatAccent = pilotAccent === "theme" ? ownAccent : ACCENTS[pilotAccent][scheme];
  const choose = useCallback(
    (text: string) => (conversationId ? sendMessage(conversationId, text) : Promise.resolve()),
    [conversationId, sendMessage],
  );
  // A reset or delete, then the catalog read again: a deleted Pilot's screen
  // leaves for the start once it's gone from there.
  const act = async (run: () => Promise<unknown>) => {
    try {
      await run();
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
      await refresh();
    } catch (reason) {
      Alert.alert("Couldn’t do that", reason instanceof Error ? reason.message : String(reason));
    }
  };
  if (!pilot || !conversationId) return <Redirect href="/" />;

  const openDetails = () =>
    router.push({ pathname: "/pilot-profile/[pilotId]", params: { pilotId: pilot.id } });
  // Both clear what can't come back, so each asks first, as the desktop does.
  const confirmReset = () =>
    Alert.alert(
      "Reset conversation?",
      `The chat with ${pilot.name} will be cleared. Its soul, memory and habits stay.`,
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Reset",
          style: "destructive",
          onPress: () => void act(() => command("pilot.reset", { pilotId: pilot.id })),
        },
      ],
    );
  const confirmDelete = () =>
    Alert.alert(
      `Delete ${pilot.name}?`,
      "Its scheduled habits stop. Sessions and notes it created stay saved.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: () => void act(() => command("pilot.delete", { pilotId: pilot.id })),
        },
      ],
    );
  const transcript = (
    <AccentOverride value={chatAccent}>
      <PilotChoose value={choose}>
        <PilotMessageActions value={messageActions}>
          {hasNativeTranscript
            ? <NativeTranscript key={pilot.id} conversationId={conversationId} agent={agent} />
            : <PilotTranscript key={pilot.id} conversationId={conversationId} agent={agent} />}
        </PilotMessageActions>
      </PilotChoose>
    </AccentOverride>
  );

  return (
    <Timed id="pilot screen">
    <SessionScreen
      pill={{
        icon: <PixelMascot name={pilot.mascot} color={pilot.color} status={headerStatus(pilot)} size={28} />,
        label: pilot.name,
        // Its profile: the desktop's details, and its look to edit.
        onPress: tools
          ? () => router.push({ pathname: "/pilot-profile/[pilotId]", params: { pilotId: pilot.id } })
          : undefined,
      }}
      // Without them, settings on the right, as on a project.
      action={tools ? undefined : { icon: "settings", label: "Settings", onPress: () => router.push("/settings") }}
      // Its details, a fresh start or deleting it, from ⋯ on the right.
      trailing={
        tools ? (
          <PilotMenu onDetails={openDetails} onReset={confirmReset} onDelete={confirmDelete} />
        ) : undefined
      }
      composer={
        <AccentOverride value={chatAccent}>
          {conversation.question ? (
            <QuestionCard conversationId={conversationId} prompt={conversation.question} />
          ) : null}
          {conversationId && conversation.usageLimit ? (
            <UsageLimitCard key={`usage:${conversationId}`} conversationId={conversationId} state={conversation.usageLimit} />
          ) : null}
          <Composer
            key={pilot.id}
            // One line: the header names the Pilot, and its model lives in settings.
            active={focused}
            compact
            context=""
            branch=""
            model={model}
            effort={effort}
            harness={harness}
            onModelPress={() =>
              router.push({
                pathname: "/models",
                params: { projectId: "", sessionId: conversationId },
              })
            }
            busy={conversation.busy}
            onStop={() => stop(conversationId)}
            reply={reply}
            onClearReply={() => setReply(null)}
            onSend={(text, attachments) => {
              // A reply opens with the message it answers, quoted on one line,
              // so the Pilot reads it in context and the chat shows it above.
              const quoted = reply?.replace(/\s+/g, " ").trim();
              setReply(null);
              return sendMessage(
                conversationId,
                quoted ? `> ${quoted.length > 280 ? quoted.slice(0, 279) + "…" : quoted}\n\n${text}` : text,
                attachments,
              );
            }}
          />
        </AccentOverride>
      }
    >
      <ConnectionNotice error={conversation.error} />
      {pilotOnly ? (
        <>
          {/* The stack's native header: back from the stack, the Pilot's
              pill for a title, and its ⋯ as a native menu. */}
          <Stack.Screen
            options={{
              headerTitle: () => (
                <PilotTitle pilot={pilot} onPress={tools ? openDetails : undefined} />
              ),
            }}
          />
          <Stack.Toolbar placement="right">
            {tools ? (
              <Stack.Toolbar.Menu icon="ellipsis" accessibilityLabel="More">
                <Stack.Toolbar.MenuAction icon="info.circle" onPress={openDetails}>
                  View details
                </Stack.Toolbar.MenuAction>
                <Stack.Toolbar.MenuAction icon="arrow.counterclockwise" onPress={confirmReset}>
                  Reset conversation
                </Stack.Toolbar.MenuAction>
                <Stack.Toolbar.MenuAction icon="trash" destructive onPress={confirmDelete}>
                  Delete Pilot
                </Stack.Toolbar.MenuAction>
              </Stack.Toolbar.Menu>
            ) : (
              <Stack.Toolbar.Button
                icon="gearshape"
                accessibilityLabel="Settings"
                onPress={() => router.push("/settings")}
              />
            )}
          </Stack.Toolbar>
          <SessionContent>{transcript}</SessionContent>
        </>
      ) : (
        // A suggested reply it offers is sent as the user's message.
        transcript
      )}
    </SessionScreen>
    </Timed>
  );
}

/** A Pilot's header mascot holds still while it works: its chat shows that,
 *  beside the "Thinking…" line. The bulb still asks for you. */
function headerStatus(pilot: Pilot) {
  return pilot.status === "working" ? "idle" : pilot.status;
}

/** A pushed Pilot's title: its mascot and name in a glass pill, which opens
 *  its profile. */
function PilotTitle({ pilot, onPress }: { pilot: Pilot; onPress?: () => void }) {
  const styles = useTitleStyles();
  return (
    <LiquidView
      borderRadius={HEADER_HEIGHT / 2}
      style={styles.frame}
      innerStyle={styles.pill}
      onPress={onPress}
      accessibilityLabel={onPress ? pilot.name : undefined}
      pressedScale={onPress ? 1.04 : undefined}
    >
      <PixelMascot name={pilot.mascot} color={pilot.color} status={headerStatus(pilot)} size={28} />
      <Text style={styles.label} numberOfLines={1}>
        {pilot.name}
      </Text>
    </LiquidView>
  );
}

const useTitleStyles = themedStyles((colors) => ({
  // Kept narrow so a long name truncates rather than spanning the header.
  frame: {
    maxWidth: 220,
  },
  pill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    height: HEADER_HEIGHT,
    paddingLeft: 12,
    paddingRight: 18,
  },
  label: {
    flexShrink: 1,
    color: colors.content,
    fontSize: 17,
    fontWeight: "600",
  },
}));

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
    <EmptySession pilot={agent} />
  );
}
