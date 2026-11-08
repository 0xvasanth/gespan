import { Host, HStack, Label, Menu, Picker, Spacer } from "@expo/ui/swift-ui";
import {
  accessibilityLabel,
  contentShape,
  frame,
  pickerStyle,
  shapes,
  tag,
} from "@expo/ui/swift-ui/modifiers";
import { router, useGlobalSearchParams, usePathname } from "expo-router";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Image, Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import Animated from "react-native-reanimated";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { useConnection, usePilotTools } from "@/features/connection/ConnectionProvider";
import { PixelMascot } from "@/features/mascots/PixelMascot";
import { ProjectMascot } from "@/features/mascots/ProjectMascot";
import { plainText } from "@/features/sessions/markdownParser";
import { HEADER_GAP, HEADER_HEIGHT, useKeyboardLift } from "@/features/sessions/SessionScreen";
import { GESPAN_MARK_COLOR } from "@/ui/brand";
import { ChoiceMenu } from "@/ui/ChoiceMenu";
import { CircleButton } from "@/ui/CircleButton";
import { useWarmDitherLoading } from "@/ui/DitherLoading";
import { GradualBlur } from "@/ui/GradualBlur";
import { Icon, type IconName } from "@/ui/Icon";
import { LiquidView } from "@/ui/LiquidView";
import { isModalRoute } from "@/ui/SceneFade";
import { AnimationsPaused } from "@/ui/AnimationsPaused";
import { useDrawerActions, useDrawerOpen } from "@/ui/ScenePager";
import { Shimmer } from "@/ui/Shimmer";
import { radii, themedStyles, useColors, useScheme } from "@/ui/theme";

import { useUnreadPilots } from "./pilotSeen";
import { onPilotCreated } from "./newPilot";
import { pickProject } from "./projectPick";
import { projectForRoute } from "./routing";
import type { Pilot, Project } from "./types";
import { useWorkspace } from "./WorkspaceProvider";

/** As the composer's pill, so the drawer's foot lines up with a chat's. */
const FOOTER_HEIGHT = 50;
/** How far past the header and footer the blur keeps fading. */
const FADE = 16;
/** The Gespan mark in the header. */
const MARK = 38;
/** A Pilot row's mascot, as a chat list's avatar. */
const AVATAR = 40;
/** The rows' inset and their own padding, so a Pilot's figure lines up with
 *  the header's buttons. */
const LIST_INSET = 6;
const ROW_INSET = 10;
const ROW_GAP = 14;

type Filter = "all" | "unread" | "working";
/** Which kinds the list shows. */
type Show = "both" | "pilots" | "projects";

/**
 * The phone's counterpart of the desktop's rail, laid out as a messaging
 * app's chat list: the Gespan mark, the filter and a new Pilot up top, level
 * with a screen's header; the Pilots with their latest line and when; and at
 * Projects under their own label, and at the foot settings and search, level
 * with a screen's composer. Picking one switches the screen under it as the
 * drawer shuts.
 */
export function WorkspaceDrawer() {
  const drawer = useDrawerActions();
  const open = useDrawerOpen();
  // The switch's loading cover, drawn while the drawer is open to be picked from.
  useWarmDitherLoading(open);
  const insets = useSafeAreaInsets();
  const colors = useColors();
  const styles = useStyles();
  const { projects, pilots: roster } = useWorkspace();
  const { pairing } = useConnection();
  const tools = usePilotTools();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [show, setShow] = useState<Show>("both");
  const unread = useUnreadPilots(pairing?.desktop.fingerprint, roster);
  const pathname = usePathname();
  const params = useGlobalSearchParams<{ projectId?: string; pilotId?: string }>();
  const onPilot = pathname.startsWith("/pilot/");
  const onProject = pathname.startsWith("/project/");
  const activePilot = onPilot ? params.pilotId : undefined;
  const activeProject = onProject
    ? projectForRoute(projects, params.projectId)?.id
    : undefined;

  // The pilot heard from last first, as a messaging app's chat list, narrowed
  // by the search and the filter.
  const needle = query.trim().toLowerCase();
  const pilots = useMemo(
    () =>
      [...roster]
        .sort((a, b) => (b.lastActiveAt || 0) - (a.lastActiveAt || 0))
        .filter(
          (pilot) =>
            (!needle || pilot.name.toLowerCase().includes(needle)) &&
            (filter === "all" ||
              (filter === "unread" ? unread.has(pilot.id) : pilot.status !== "idle")),
        ),
    [roster, needle, filter, unread],
  );
  const shownProjects = projects.filter(
    (project) =>
      (!needle || project.name.toLowerCase().includes(needle)) &&
      // A project has no replies to read, so only "All" and "Working now" keep it.
      (filter === "all" || (filter === "working" && project.busy)),
  );

  // The drawer shuts at once over a loading cover, and the next screen is
  // built only once it is shut, so building it can't hitch the motion. Same
  // kind to same kind only swaps the params, so the frame stays mounted.
  const openProject = (id: string) => {
    if (id === activeProject) return drawer.close();
    drawer.closeThen(() => {
      // The project route takes a pick in place; any other screen is replaced.
      if (!pickProject(id))
        router.replace({ pathname: "/project/[projectId]", params: { projectId: id } });
    });
  };
  const openPilot = (id: string) => {
    if (id === activePilot) return drawer.close();
    // Pilot to Pilot only swaps the conversation in the same screen, which is
    // light enough to switch as the drawer shuts.
    if (onPilot) {
      drawer.close();
      return router.setParams({ pilotId: id });
    }
    drawer.closeThen(() =>
      router.replace({ pathname: "/pilot/[pilotId]", params: { pilotId: id } }),
    );
  };
  // Settings rises over the open drawer, which is still there once it's shut.
  const openSettings = () => {
    if (!isModalRoute(pathname)) router.push("/settings");
  };
  // A new Pilot from its own sheet, which rises over the open drawer as
  // settings does; once made, the drawer shuts onto its chat.
  const createPilot = () => {
    if (!isModalRoute(pathname)) router.push("/pilot-new");
  };
  const openCreated = useRef(openPilot);
  useEffect(() => {
    openCreated.current = openPilot;
  });
  useEffect(() => onPilotCreated((pilotId) => openCreated.current(pilotId)), []);

  // Each kind under its label, as the filter lets it show.
  const listedPilots = show === "projects" ? [] : pilots;
  const listedProjects = show === "pilots" ? [] : shownProjects;
  const headerTop = insets.top + HEADER_GAP;
  const headerBottom = headerTop + HEADER_HEIGHT + 12;
  const footerBottom = Math.max(insets.bottom, 12);
  const footerTop = footerBottom + FOOTER_HEIGHT + 12;
  // The search rides the keyboard, resting just above it.
  const lift = useKeyboardLift(footerBottom - 10);
  const nothing = !listedPilots.length && !listedProjects.length;

  return (
    // A shut drawer's mascots hold still; they move again as it opens.
    <AnimationsPaused value={!open}>
      <View style={styles.container}>
        <ScrollView
          style={StyleSheet.absoluteFill}
          contentContainerStyle={{ paddingTop: headerBottom, paddingBottom: footerTop + FADE }}
          scrollIndicatorInsets={{ top: headerBottom, bottom: footerTop }}
          showsVerticalScrollIndicator={false}
          keyboardShouldPersistTaps="handled"
          keyboardDismissMode="on-drag"
        >
          {listedPilots.length ? (
            <View style={styles.list}>
              <Text style={styles.sectionLabel}>Pilots</Text>
              {listedPilots.map((pilot) => (
                <PilotRow
                  key={pilot.id}
                  pilot={pilot}
                  unread={unread.has(pilot.id)}
                  active={pilot.id === activePilot}
                  onPress={openPilot}
                />
              ))}
            </View>
          ) : null}
          {listedProjects.length ? (
            <View style={[styles.list, listedPilots.length > 0 && styles.section]}>
              <Text style={styles.sectionLabel}>Projects</Text>
              {listedProjects.map((project) => (
                <ProjectRow
                  key={project.id}
                  project={project}
                  active={project.id === activeProject}
                  onPress={openProject}
                />
              ))}
            </View>
          ) : null}
          {nothing ? (
            <Text style={styles.empty}>
              {needle || filter !== "all" || show !== "both"
                ? "Nothing matches."
                : "No Pilots or projects on this phone yet."}
            </Text>
          ) : null}
        </ScrollView>

        <GradualBlur edge="top" height={headerBottom + FADE} color={colors.drawer} />
        <GradualBlur edge="bottom" height={footerTop + FADE} color={colors.drawer} />

        <View style={[styles.header, { top: headerTop }]}>
          <Image
            source={GESPAN_MARK_COLOR}
            style={styles.mark}
            accessibilityIgnoresInvertColors
          />
          <View style={styles.headerActions}>
            <FilterMenu filter={filter} onFilter={setFilter} show={show} onShow={setShow} />
            {/* The older desktop app can't make a Pilot from the phone. */}
            {tools ? <CircleButton icon="add" label="New Pilot" onPress={createPilot} /> : null}
          </View>
        </View>

        <Animated.View style={[styles.foot, { bottom: footerBottom }, lift]}>
          <View style={styles.footer}>
          <CircleButton icon="settings" label="Settings" onPress={openSettings} size={FOOTER_HEIGHT} />
          <LiquidView borderRadius={FOOTER_HEIGHT / 2} style={styles.searchFrame} innerStyle={styles.search}>
            <Icon name="search" size={18} color={colors.muted} />
            <TextInput
              value={query}
              onChangeText={setQuery}
              placeholder="Search"
              placeholderTextColor={colors.muted}
              keyboardAppearance={useScheme()}
              returnKeyType="search"
              autoCorrect={false}
              clearButtonMode="while-editing"
              style={styles.searchInput}
            />
          </LiquidView>
          </View>
        </Animated.View>
      </View>
    </AnimationsPaused>
  );
}

// Each with its SF Symbol for the iOS menu and its icon for Android's sheet.
const FILTERS: {
  value: Filter;
  label: string;
  symbol: "tray.full" | "circlebadge" | "bolt";
  icon: IconName;
}[] = [
  { value: "all", label: "All", symbol: "tray.full", icon: "inbox" },
  { value: "unread", label: "Unread", symbol: "circlebadge", icon: "circle" },
  { value: "working", label: "Working now", symbol: "bolt", icon: "automation" },
];

const SHOWS: {
  value: Show;
  label: string;
  symbol: "square.stack" | "person.2" | "folder";
  icon: IconName;
}[] = [
  { value: "both", label: "Pilots and projects", symbol: "square.stack", icon: "layers" },
  { value: "pilots", label: "Pilots", symbol: "person.2", icon: "people" },
  { value: "projects", label: "Projects", symbol: "folder", icon: "folder" },
];

/** The round button beside a new Pilot: a native menu of which kinds to show
 *  and which of them. Lit while either narrows the list. */
function FilterMenu({
  filter,
  onFilter,
  show,
  onShow,
}: {
  filter: Filter;
  onFilter: (value: Filter) => void;
  show: Show;
  onShow: (value: Show) => void;
}) {
  const colors = useColors();
  const on = filter !== "all" || show !== "both";
  // The SwiftUI menu has no Android view, so Android groups the same choices in a sheet.
  if (Platform.OS === "android") return (
    <ChoiceMenu label={on ? "Filter, on" : "Filter"} title="Filter" sections={[
      SHOWS.map((option) => ({
        label: option.label,
        icon: option.icon,
        selected: show === option.value,
        onPress: () => onShow(option.value),
      })),
      FILTERS.map((option) => ({
        label: option.label,
        icon: option.icon,
        selected: filter === option.value,
        onPress: () => onFilter(option.value),
      })),
    ]}>
      <LiquidView borderRadius={HEADER_HEIGHT / 2} innerStyle={filterStyles.round}>
        <Icon name="filter" size={20} color={on ? colors.accent : colors.content} />
      </LiquidView>
    </ChoiceMenu>
  );
  return (
    <View>
      <LiquidView borderRadius={HEADER_HEIGHT / 2} innerStyle={filterStyles.round}>
        <Icon name="filter" size={20} color={on ? colors.accent : colors.content} />
      </LiquidView>
      {/* The native menu lies over the glass, its label an invisible box of
          the button's size, as in SessionFilterMenu. */}
      <Host style={StyleSheet.absoluteFill}>
        <Menu
          modifiers={[accessibilityLabel(on ? "Filter, on" : "Filter")]}
          label={
            <HStack modifiers={[frame({ width: HEADER_HEIGHT, height: HEADER_HEIGHT }), contentShape(shapes.circle())]}>
              <Spacer />
            </HStack>
          }
        >
          <Picker
            selection={show}
            onSelectionChange={(next) => onShow(next as Show)}
            modifiers={[pickerStyle("inline")]}
          >
            {SHOWS.map((option) => (
              <Label key={option.value} title={option.label} systemImage={option.symbol} modifiers={[tag(option.value)]} />
            ))}
          </Picker>
          <Picker
            selection={filter}
            onSelectionChange={(next) => onFilter(next as Filter)}
            modifiers={[pickerStyle("inline")]}
          >
            {FILTERS.map((option) => (
              <Label key={option.value} title={option.label} systemImage={option.symbol} modifiers={[tag(option.value)]} />
            ))}
          </Picker>
        </Menu>
      </Host>
    </View>
  );
}

const filterStyles = StyleSheet.create({
  round: {
    width: HEADER_HEIGHT,
    height: HEADER_HEIGHT,
  },
});

/** A Pilot as a chat in a messaging app's list: its mascot, its name and when
 *  it was last active, and under them what it is doing or its last line. */
const PilotRow = memo(function PilotRow({
  pilot,
  unread,
  active,
  onPress,
}: {
  pilot: Pilot;
  /** A reply it sent that the user hasn't seen. */
  unread: boolean;
  active: boolean;
  onPress: (id: string) => void;
}) {
  const styles = useStyles();
  // The shimmer only runs where it can be seen.
  const shown = useDrawerOpen();
  const detail =
    pilot.status === "working"
      ? "Working…"
      : pilot.status === "needs-you"
        ? "Needs you"
        : oneLine(pilot.lastMessage) || "No messages yet";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${pilot.name}, ${detail}`}
      accessibilityState={{ selected: active }}
      onPress={() => onPress(pilot.id)}
      style={({ pressed }) => [styles.row, (active || pressed) && styles.rowActive]}
    >
      <PixelMascot name={pilot.mascot} color={pilot.color} status={pilot.status} size={AVATAR} centered />
      <View style={styles.rowText}>
        <View style={styles.rowTop}>
          <Text style={styles.rowLabel} numberOfLines={1}>
            {pilot.name}
          </Text>
          {pilot.lastActiveAt ? (
            <Text style={[styles.rowTime, unread && styles.rowTimeUnread]}>{when(pilot.lastActiveAt)}</Text>
          ) : null}
        </View>
        <View style={styles.rowTop}>
          {pilot.status === "working" && shown ? (
            <Shimmer style={[styles.rowDetail, styles.rowDetailFill]}>{detail}</Shimmer>
          ) : (
            <Text
              style={[styles.rowDetail, styles.rowDetailFill, pilot.status === "needs-you" && styles.rowNeedsYou]}
              numberOfLines={1}
            >
              {detail}
            </Text>
          )}
          {unread ? <View style={styles.unread} /> : null}
        </View>
      </View>
    </Pressable>
  );
});

/** When a Pilot was last active, as a chat list dates its rows: the time
 *  today, Yesterday, else the day. */
export function when(at: number) {
  const date = new Date(at);
  const today = new Date();
  if (date.toDateString() === today.toDateString())
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (date.toDateString() === yesterday.toDateString()) return "Yesterday";
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}

/** The start of a message as one plain line: its markdown read for its
 *  words, so **bold** shows as bold rather than its asterisks. Only as much
 *  as one line can show is read. */
export function oneLine(text: string) {
  // A reply that is only a reaction, as the chat shows it, by its emoji.
  const reaction = text.trim().match(/^<reaction>\s*(.{1,16}?)\s*<\/reaction>$/su);
  if (reaction) return `Reacted ${reaction[1]}`;
  return plainText(text.slice(0, 600));
}

/** A project as a row of the list, under the Pilots: its mascot and name. */
const ProjectRow = memo(function ProjectRow({
  project,
  active,
  onPress,
}: {
  project: Project;
  active: boolean;
  onPress: (id: string) => void;
}) {
  const styles = useStyles();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={project.name}
      accessibilityState={{ selected: active }}
      onPress={() => onPress(project.id)}
      style={({ pressed }) => [styles.projectRow, (active || pressed) && styles.rowActive]}
    >
      <View style={styles.projectIcon}>
        <ProjectMascot
          project={project.path}
          name={project.mascot}
          color={project.color}
          size={24}
          active={project.busy}
        />
      </View>
      <Text style={[styles.rowLabel, styles.rowText]} numberOfLines={1}>
        {project.name}
      </Text>
    </Pressable>
  );
});

const useStyles = themedStyles((colors) => ({
  container: {
    flex: 1,
    backgroundColor: colors.drawer,
  },
  header: {
    position: "absolute",
    left: 0,
    right: 0,
    height: HEADER_HEIGHT,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 16,
  },
  headerActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  // The Gespan mark, centred on a row's figure below it.
  mark: {
    width: MARK,
    height: MARK,
    marginLeft: LIST_INSET + ROW_INSET + (AVATAR - MARK) / 2 - 16,
  },
  foot: {
    position: "absolute",
    left: 0,
    right: 0,
  },
  section: {
    marginTop: 18,
  },
  // Dimmed heading over each kind, inset like the rows' text.
  sectionLabel: {
    paddingHorizontal: ROW_INSET,
    marginBottom: 4,
    color: colors.muted,
    fontSize: 13,
    fontWeight: "500",
  },
  pressed: {
    opacity: 0.6,
  },
  list: {
    paddingHorizontal: LIST_INSET,
    gap: 2,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: ROW_GAP,
    minHeight: 64,
    paddingLeft: ROW_INSET,
    // Clear of the scene's edge, which the drawer meets on the right.
    paddingRight: ROW_INSET + 12,
    paddingVertical: 10,
    borderRadius: radii.card - 6,
    borderCurve: "continuous",
  },
  projectRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: ROW_GAP,
    minHeight: 52,
    paddingHorizontal: ROW_INSET,
    paddingVertical: 8,
    borderRadius: radii.row,
    borderCurve: "continuous",
  },
  // The width of a Pilot's figure, so the logo above centres on it either way.
  projectIcon: {
    width: AVATAR,
    alignItems: "center",
  },
  rowActive: {
    backgroundColor: colors.selection,
  },
  rowText: {
    flex: 1,
    minWidth: 0,
    gap: 3,
  },
  rowTop: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  rowLabel: {
    flex: 1,
    color: colors.content,
    fontSize: 17,
    fontWeight: "600",
  },
  rowTime: {
    color: colors.muted,
    fontSize: 14,
    fontVariant: ["tabular-nums"],
  },
  rowTimeUnread: {
    color: colors.accent,
  },
  rowDetail: {
    color: colors.muted,
    fontSize: 15,
  },
  rowDetailFill: {
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
    paddingHorizontal: LIST_INSET + ROW_INSET,
    paddingTop: 12,
    color: colors.muted,
    fontSize: 15,
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
