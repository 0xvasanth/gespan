import * as Haptics from "expo-haptics";
import type { ReactNode } from "react";
import { Pressable, Text, View } from "react-native";
import type { SharedValue } from "react-native-reanimated";

import { LiquidView } from "./LiquidView";
import { themedStyles } from "./theme";

/**
 * Android's native header can miss layout events for children inside clipped
 * animated label windows. Render the selected label at its natural size so
 * neither it nor its highlight depends on measurements or cached widths.
 * Page swipes update `value` through the pager, just as taps do.
 */
export function SegmentedSwitch<T extends string>({
  segments,
  value,
  onChange,
}: {
  segments: { value: T; label: string; icon: ReactNode }[];
  value: T;
  onChange: (value: T) => void;
  progress?: SharedValue<number>;
}) {
  const styles = useStyles();
  const selectedIndex = Math.max(0, segments.findIndex((segment) => segment.value === value));
  return (
    <LiquidView style={styles.compact} innerStyle={styles.track}>
      {segments.map((segment, index) => {
        const selected = index === selectedIndex;
        return (
          <Pressable
            key={segment.value}
            accessibilityRole="tab"
            accessibilityLabel={segment.label}
            accessibilityState={{ selected }}
            onPress={() => {
              if (selected) return;
              void Haptics.selectionAsync();
              onChange(segment.value);
            }}
            style={[styles.segment, selected && styles.selected]}
          >
            <View style={{ opacity: selected ? 1 : 0.5 }}>{segment.icon}</View>
            {selected ? <Text numberOfLines={1} style={styles.label}>{segment.label}</Text> : null}
          </Pressable>
        );
      })}
    </LiquidView>
  );
}

const useStyles = themedStyles((colors) => ({
  compact: { alignSelf: "center" },
  track: { height: 44, padding: 4 },
  segment: {
    height: 36,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 12,
    gap: 8,
    borderRadius: 18,
  },
  selected: { paddingHorizontal: 16, backgroundColor: colors.thumb },
  label: {
    maxWidth: 120,
    color: colors.content,
    fontSize: 15,
    fontWeight: "600",
  },
}));
