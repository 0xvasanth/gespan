import * as Haptics from "expo-haptics";
import { Link, router, Stack, useIsFocused, useNavigation } from "expo-router";
import {
  type ComponentRef,
  memo,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Alert, Image, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import Animated, {
  FadeIn,
  FadeOut,
  LayoutAnimationConfig,
  LinearTransition,
} from "react-native-reanimated";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { useConnection, usePilotTools } from "@/features/connection/ConnectionProvider";
import { hideScrollEdgeEffects } from "@/features/connection/nativeLink";
import { PixelMascot } from "@/features/mascots/PixelMascot";
import { HeaderBlur } from "@/features/files/codeScreen";
import { plainText } from "@/features/sessions/markdownParser";
import { useKeyboardLift } from "@/features/sessions/SessionScreen";
import { GESPAN_MARK } from "@/ui/brand";
import { CircleButton } from "@/ui/CircleButton";
import { GradualBlur } from "@/ui/GradualBlur";
import { Icon } from "@/ui/Icon";
import { LiquidView } from "@/ui/LiquidView";
import { Shimmer } from "@/ui/Shimmer";
import { themedStyles, useColors, useScheme } from "@/ui/theme";

import { markPilotSeen, markPilotUnread, useUnreadPilots } from "./pilotSeen";
import { onPilotCreated } from "./newPilot";
import type { Pilot } from "./types";
import { oneLine, when } from "./WorkspaceDrawer";
import { useWorkspace } from "./WorkspaceProvider";

/** The list's tag, for turning off iOS's edge effect on it. */
const LIST_ID = "pilot-list";
/** As the composer's pill, so the list's foot lines up with a chat's. */
const FOOTER_HEIGHT = 50;
/** How far past the header and footer the blur keeps fading. */
const FADE = 16;
/** A Pilot's mascot, as a messaging app's avatar. */
const AVATAR = 44;
/** The mascot itself, a little smaller, its left edge level with the mark's. */
const MASCOT = 36;
/** How long rows take to glide to new places, and a leaving one to fade. */
const MOVE_MS = 280;
const FADE_OUT_MS = 180;
/** A long press's preview of a Pilot's latest message: iOS's preferred size. */
const PREVIEW_WIDTH = 340;
const PREVIEW_HEIGHT = 260;

/**
 * The Pilots-only layout's first screen, as a messaging app's chat list:
 * the Gespan mark and a new Pilot in the native header, the Pilots heard
 * from last first, and settings and search in glass at the foot, level with a chat's
 * composer. A Pilot's chat pushes over it.
 */
export function PilotHome() {
  const insets = useSafeAreaInsets();
  const colors = useColors();
  const styles = useStyles();
  const { pilots: roster } = useWorkspace();
  const { pairing, command, refresh, selection, select } = useConnection();
  const tools = usePilotTools();
  const [query, setQuery] = useState("");
  const [listReady, setListReady] = useState(false);
  // iOS's edge effect, a band with a hard line, off under the header once
  // the list is in: the stack misses this list, so it's done directly.
  useEffect(() => {
    if (!listReady) return;
    hideScrollEdgeEffects(LIST_ID);
    // Again a moment later, in case the stack set it back meanwhile.
    const timer = setTimeout(() => hideScrollEdgeEffects(LIST_ID), 500);
    return () => clearTimeout(timer);
  }, [listReady]);
  // A Pilot made in the new-Pilot sheet opens once the sheet is gone and the
  // list is in front again, so its push doesn't meet the sheet's dismissal.
  const created = useRef<string | null>(null);
  useEffect(
    () =>
      onPilotCreated((pilotId) => {
        created.current = pilotId;
      }),
    [],
  );
  const unread = useUnreadPilots(pairing?.desktop.fingerprint, roster);
  // Its rows' shimmer runs only while the list is in front.
  const focused = useIsFocused();
  // Whether a chat has finished sliding over the list; it is uncovered again
  // as soon as the list is back in front.
  const navigation = useNavigation();
  const [coveredByChat, setCovered] = useState(false);
  useEffect(
    () =>
      // As the chat starts to slide over: iOS takes the list off screen the
      // moment the push ends, and with the blurs still on it that stalls.
      navigation.addListener("transitionStart" as never, (event: { data?: { closing?: boolean } }) => {
        if (event.data?.closing) setCovered(true);
      }),
    [navigation],
  );
  const covered = coveredByChat && !focused;
  if (coveredByChat && focused) setCovered(false);

  // Pilots deleted or hidden here leave the list at once, before the desktop
  // has answered; one it refuses comes back.
  const [gone, setGone] = useState<ReadonlySet<string>>(() => new Set());
  const setGoneId = (id: string, isGone: boolean) =>
    setGone((previous) => {
      const next = new Set(previous);
      if (isGone) next.add(id);
      else next.delete(id);
      return next;
    });

  const needle = query.trim().toLowerCase();
  const pilots = useMemo(
    () =>
      [...roster]
        .sort((a, b) => (b.lastActiveAt || 0) - (a.lastActiveAt || 0))
        .filter(
          (pilot) =>
            !gone.has(pilot.id) && (!needle || pilot.name.toLowerCase().includes(needle)),
        ),
    [roster, needle, gone],
  );

  useEffect(() => {
    const pilotId = created.current;
    if (!focused || !pilotId) return;
    created.current = null;
    openPilot(pilotId);
  }, [focused]);

  // A row's long-press menu. Reset and delete clear what can't come back, so
  // each asks first, as the desktop does.
  const desktop = pairing?.desktop.fingerprint;
  const act = async (run: () => Promise<unknown>) => {
    try {
      await run();
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
      await refresh();
    } catch (reason) {
      Alert.alert("Couldn’t do that", reason instanceof Error ? reason.message : String(reason));
    }
  };
  // Out of the list now, and back if the desktop refuses.
  const removeNow = (pilot: Pilot, run: () => Promise<unknown>) => {
    void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    setGoneId(pilot.id, true);
    run()
      .then(() => refresh())
      .catch((reason: unknown) => {
        setGoneId(pilot.id, false);
        Alert.alert("Couldn’t do that", reason instanceof Error ? reason.message : String(reason));
      });
  };
  const handlers: RowActions = {
    toggleRead: (pilot) =>
      unread.has(pilot.id) ? markPilotSeen(desktop, pilot) : markPilotUnread(desktop, pilot),
    details: tools
      ? (pilot) => router.push({ pathname: "/pilot-profile/[pilotId]", params: { pilotId: pilot.id } })
      : undefined,
    reset: tools
      ? (pilot) =>
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
          )
      : undefined,
    // Off this phone only; it comes back from Workspace in settings.
    hide: selection
      ? (pilot) =>
          removeNow(pilot, () =>
            select({ ...selection, pilotIds: selection.pilotIds.filter((id) => id !== pilot.id) }),
          )
      : undefined,
    remove: tools
      ? (pilot) =>
          Alert.alert(
            `Delete ${pilot.name}?`,
            "Its scheduled habits stop. Sessions and notes it created stay saved.",
            [
              { text: "Cancel", style: "cancel" },
              {
                text: "Delete",
                style: "destructive",
                onPress: () => removeNow(pilot, () => command("pilot.delete", { pilotId: pilot.id })),
              },
            ],
          )
      : undefined,
  };
  const latest = useRef<RowActions | null>(null);
  useEffect(() => {
    latest.current = handlers;
  });
  // Stable, so the memoized rows don't re-render for it; each call reads the
  // latest handlers.
  const [actions] = useState<RowActions>(() => ({
    toggleRead: (pilot) => latest.current?.toggleRead(pilot),
    details: (pilot) => latest.current?.details?.(pilot),
    reset: (pilot) => latest.current?.reset?.(pilot),
    hide: (pilot) => latest.current?.hide?.(pilot),
    remove: (pilot) => latest.current?.remove?.(pilot),
  }));

  const footerBottom = Math.max(insets.bottom, 12);
  const footerTop = footerBottom + FOOTER_HEIGHT + 12;
  // The search rides its own keyboard, resting just above it.
  const search = useRef<ComponentRef<typeof TextInput>>(null);
  const lift = useKeyboardLift(footerBottom - 10, search);
  const scheme = useScheme();

  return (
    <View style={styles.container}>
      {/* First, so it leads the screen's first-child chain: that is where
          iOS looks for the scroll view whose edge effect is turned off
          under the header, leaving our own gradual blur. */}
      <ScrollView
        testID={LIST_ID}
        onLayout={listReady ? undefined : () => setListReady(true)}
        onScrollBeginDrag={() => hideScrollEdgeEffects(LIST_ID)}
        style={StyleSheet.absoluteFill}
        // Clear of the native header, which it runs under, and of the foot.
        contentInsetAdjustmentBehavior="automatic"
        contentContainerStyle={{ paddingTop: 4, paddingBottom: footerTop + FADE }}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode="on-drag"
      >
        {/* Rows glide to their new places as one leaves or a new message
            brings one to the top, and fade as they go or come; the first
            ones in are simply there. */}
        <LayoutAnimationConfig skipEntering>
          {pilots.map((pilot) => (
            <Animated.View
              key={pilot.id}
              layout={LinearTransition.duration(MOVE_MS)}
              entering={FadeIn.duration(MOVE_MS)}
              exiting={FadeOut.duration(FADE_OUT_MS)}
            >
              <PilotRow
                pilot={pilot}
                unread={unread.has(pilot.id)}
                shown={focused}
                tools={tools}
                actions={actions}
              />
            </Animated.View>
          ))}
        </LayoutAnimationConfig>
        {pilots.length ? null : (
          <View style={styles.empty}>
            <Text style={styles.emptyText}>
              {needle ? "Nothing matches." : "No Pilots on this phone yet."}
            </Text>
            {needle ? null : (
              <Text style={styles.emptyLink} onPress={() => router.push("/settings/workspace")}>
                Choose Pilots
              </Text>
            )}
          </View>
        )}
      </ScrollView>

      {/* The stack's native header: the Gespan mark and a new Pilot as
          native items, so they morph into a chat's back and ⋯ as one is
          pushed. */}
      <Stack.Screen options={{ headerTitle: "" }} />
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.View hidesSharedBackground>
          <View style={styles.markBox}>
            <Image
              source={GESPAN_MARK}
              style={[styles.mark, { tintColor: colors.content }]}
              accessibilityLabel="Gespan"
              accessibilityIgnoresInvertColors
            />
          </View>
        </Stack.Toolbar.View>
      </Stack.Toolbar>
      {/* The older desktop app can't make a Pilot from the phone. */}
      {tools ? (
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button
            icon="plus"
            accessibilityLabel="New Pilot"
            onPress={() => router.push("/pilot-new")}
          />
        </Stack.Toolbar>
      ) : null}

      {/* The app's own gradual blur under the header and the foot, as
          over every screen, rather than iOS's edge effects. Not kept while
          a chat covers the list: its masked layers are rebuilt each time
          iOS takes the list off screen or puts it back, which stalls the
          slides in and out of a chat. */}
      {covered ? null : (
        <>
          <HeaderBlur color={colors.background} />
          <GradualBlur edge="bottom" height={footerTop + FADE} color={colors.background} />
        </>
      )}

      <Animated.View style={[styles.foot, { bottom: footerBottom }, lift]}>
        <View style={styles.footer}>
          <CircleButton
            icon="settings"
            label="Settings"
            onPress={() => router.push("/settings")}
            size={FOOTER_HEIGHT}
          />
          <LiquidView
            borderRadius={FOOTER_HEIGHT / 2}
            style={styles.searchFrame}
            innerStyle={styles.search}
          >
            <Icon name="search" size={18} color={colors.muted} />
            <TextInput
              ref={search}
              value={query}
              onChangeText={setQuery}
              placeholder="Search"
              placeholderTextColor={colors.muted}
              keyboardAppearance={scheme}
              returnKeyType="search"
              autoCorrect={false}
              clearButtonMode="while-editing"
              style={styles.searchInput}
            />
          </LiquidView>
        </View>
      </Animated.View>
    </View>
  );
}

/** Pushes a Pilot's chat over the list. */
function openPilot(id: string) {
  router.push({ pathname: "/pilot/[pilotId]", params: { pilotId: id } });
}

type RowActions = {
  toggleRead: (pilot: Pilot) => void;
  details?: (pilot: Pilot) => void;
  reset?: (pilot: Pilot) => void;
  hide?: (pilot: Pilot) => void;
  remove?: (pilot: Pilot) => void;
};

/** A Pilot as a chat in the list: its mascot, its name and when it was last
 *  active, and under them what it is doing or its last line. A tap opens its
 *  chat; a long press lifts it into iOS's own preview of its latest message,
 *  with a menu of what can be done to it, as Messages does. */
const PilotRow = memo(function PilotRow({
  pilot,
  unread,
  shown,
  tools,
  actions,
}: {
  pilot: Pilot;
  /** A reply it sent that the user hasn't seen. */
  unread: boolean;
  /** Whether the list is in front, for the working shimmer. */
  shown: boolean;
  /** Whether the desktop can show its details, reset or delete it. */
  tools: boolean;
  actions: RowActions;
}) {
  const styles = useStyles();
  const detail =
    pilot.status === "working"
      ? "Working…"
      : pilot.status === "needs-you"
        ? "Needs you"
        : oneLine(pilot.lastMessage) || "No messages yet";
  return (
    <Link
      href={{ pathname: "/pilot/[pilotId]", params: { pilotId: pilot.id } }}
      asChild
      onPress={() => {
        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      }}
    >
      <Link.Trigger>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`${pilot.name}, ${detail}`}
          style={styles.row}
        >
          <PixelMascot name={pilot.mascot} color={pilot.color} status={pilot.status} size={MASCOT} centered />
          <View style={styles.rowText}>
            <View style={styles.rowLine}>
              <Text style={styles.rowLabel} numberOfLines={1}>
                {pilot.name}
              </Text>
              {pilot.lastActiveAt ? (
                <Text style={[styles.rowTime, unread && styles.rowTimeUnread]}>
                  {when(pilot.lastActiveAt)}
                </Text>
              ) : null}
            </View>
            <View style={styles.rowLine}>
              {pilot.status === "working" && shown ? (
                <Shimmer style={[styles.rowDetail, styles.rowFill]}>{detail}</Shimmer>
              ) : (
                <Text
                  style={[
                    styles.rowDetail,
                    styles.rowFill,
                    pilot.status === "needs-you" && styles.rowNeedsYou,
                  ]}
                  numberOfLines={1}
                >
                  {detail}
                </Text>
              )}
              {unread ? <View style={styles.unread} /> : null}
            </View>
          </View>
        </Pressable>
      </Link.Trigger>
      <Link.Preview style={styles.preview}>
        <PilotPreview pilot={pilot} />
      </Link.Preview>
      <Link.Menu>
        <Link.MenuAction
          icon={unread ? "message" : "message.badge"}
          onPress={() => actions.toggleRead(pilot)}
        >
          {unread ? "Mark as Read" : "Mark as Unread"}
        </Link.MenuAction>
        {tools ? (
          <Link.MenuAction icon="info.circle" onPress={() => actions.details?.(pilot)}>
            View Details
          </Link.MenuAction>
        ) : null}
        {tools ? (
          <Link.MenuAction icon="arrow.counterclockwise" onPress={() => actions.reset?.(pilot)}>
            Reset Conversation
          </Link.MenuAction>
        ) : null}
        <Link.MenuAction icon="eye.slash" onPress={() => actions.hide?.(pilot)}>
          Hide
        </Link.MenuAction>
        {tools ? (
          <Link.MenuAction icon="trash" destructive onPress={() => actions.remove?.(pilot)}>
            Delete Pilot
          </Link.MenuAction>
        ) : null}
      </Link.Menu>
    </Link>
  );
});

/** What a long press lifts: the Pilot's name over its latest message, in a
 *  bubble as its chat shows it. */
function PilotPreview({ pilot }: { pilot: Pilot }) {
  const styles = useStyles();
  const text =
    pilot.status === "working"
      ? "Working…"
      : plainText(pilot.lastMessage.slice(0, 1200)).trim() || "No messages yet";
  return (
    <View style={styles.previewCard}>
      <View style={styles.previewHeader}>
        <PixelMascot name={pilot.mascot} color={pilot.color} status={pilot.status} size={28} centered />
        <Text style={styles.previewName} numberOfLines={1}>
          {pilot.name}
        </Text>
      </View>
      <View style={styles.previewBody}>
        <View style={styles.bubble}>
          <Text style={styles.bubbleText} numberOfLines={8}>
            {text}
          </Text>
        </View>
      </View>
    </View>
  );
}

const useStyles = themedStyles((colors) => ({
  container: {
    flex: 1,
    backgroundColor: colors.background,
  },
  preview: {
    width: PREVIEW_WIDTH,
    height: PREVIEW_HEIGHT,
  },
  previewCard: {
    flex: 1,
    backgroundColor: colors.background,
  },
  previewHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 18,
    paddingVertical: 14,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.stroke,
  },
  previewName: {
    flex: 1,
    color: colors.content,
    fontSize: 17,
    fontWeight: "600",
  },
  previewBody: {
    flex: 1,
    padding: 16,
  },
  bubble: {
    alignSelf: "flex-start",
    maxWidth: "88%",
    paddingHorizontal: 16,
    paddingVertical: 12,
    borderRadius: 22,
    borderCurve: "continuous",
    backgroundColor: colors.surface,
  },
  bubbleText: {
    color: colors.content,
    fontSize: 16,
    lineHeight: 22,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    minHeight: 70,
    // The mascot's left edge level with the Gespan mark's: iOS insets
    // the bar's items 16pt, and the mark sits 3pt into its 44pt box.
    paddingLeft: 19,
    paddingRight: 20,
    paddingVertical: 12,
  },
  rowText: {
    flex: 1,
    minWidth: 0,
    gap: 4,
  },
  rowLine: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  rowLabel: {
    flex: 1,
    color: colors.content,
    fontSize: 16,
    fontWeight: "500",
  },
  rowTime: {
    color: colors.muted,
    fontSize: 13,
    fontVariant: ["tabular-nums"],
  },
  rowTimeUnread: {
    color: colors.accent,
  },
  rowDetail: {
    color: colors.muted,
    fontSize: 14,
  },
  rowFill: {
    flex: 1,
  },
  rowNeedsYou: {
    color: colors.caution,
  },
  unread: {
    width: 10,
    height: 10,
    borderRadius: 5,
    backgroundColor: colors.accent,
  },
  empty: {
    paddingHorizontal: 20,
    paddingTop: 12,
    gap: 8,
  },
  emptyText: {
    color: colors.muted,
    fontSize: 15,
  },
  // The Gespan mark, centred over the rows' mascots below it.
  markBox: {
    width: AVATAR,
    height: AVATAR,
    alignItems: "center",
    justifyContent: "center",
  },
  mark: {
    width: 38,
    height: 38,
  },
  emptyLink: {
    color: colors.accent,
    fontSize: 15,
  },
  foot: {
    position: "absolute",
    left: 0,
    right: 0,
  },
  footer: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 16,
  },
  searchFrame: {
    flex: 1,
  },
  search: {
    height: FOOTER_HEIGHT,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 16,
  },
  searchInput: {
    flex: 1,
    height: FOOTER_HEIGHT,
    color: colors.content,
    fontSize: 17,
  },
}));
