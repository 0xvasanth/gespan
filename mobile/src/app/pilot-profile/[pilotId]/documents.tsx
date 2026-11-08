import { useLocalSearchParams } from "expo-router";
import { ActivityIndicator, Text, View } from "react-native";

import { SettingsPage } from "@/features/settings/SettingsUI";
import { PilotDocumentRows } from "@/features/workspace/PilotDocuments";
import { usePilotProfile } from "@/features/workspace/pilotProfile";
import { radii, themedStyles, useColors } from "@/ui/theme";

// Every document a Pilot wrote, newest first, from "See all" under the few its
// profile shows. Each opens beside it, inside the profile's sheet.
export default function PilotDocumentsScreen() {
  const { pilotId } = useLocalSearchParams<{ pilotId: string }>();
  const { profile, error, refreshing, refresh } = usePilotProfile(pilotId);
  const styles = useStyles();
  const colors = useColors();
  return (
    <SettingsPage onRefresh={refresh} refreshing={refreshing}>
      {profile ? (
        <View style={styles.card}>
          <PilotDocumentRows pilotId={pilotId} documents={profile.documents} />
        </View>
      ) : error ? (
        <Text style={styles.note}>{error}</Text>
      ) : (
        <ActivityIndicator color={colors.muted} style={styles.loading} />
      )}
    </SettingsPage>
  );
}

const useStyles = themedStyles((colors) => ({
  card: {
    backgroundColor: colors.surface,
    borderRadius: radii.card,
    borderCurve: "continuous",
    paddingHorizontal: 10,
    paddingVertical: 6,
  },
  note: {
    color: colors.muted,
    fontSize: 14,
    lineHeight: 20,
    marginHorizontal: 16,
  },
  loading: {
    marginTop: 32,
  },
}));
