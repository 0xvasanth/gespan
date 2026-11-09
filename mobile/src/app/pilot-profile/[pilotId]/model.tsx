import { router, Stack, useLocalSearchParams } from "expo-router";
import { useState } from "react";

import { ModelList } from "@/features/models/ModelPicker";
import { pilotConversationId } from "@/features/workspace/WorkspaceProvider";

// The Pilot's model, picked from every provider's, pushed inside its profile
// rather than in a sheet of its own; searched from the header.
export default function PilotModelScreen() {
  const { pilotId } = useLocalSearchParams<{ pilotId: string }>();
  const [query, setQuery] = useState("");
  return (
    <>
      <Stack.Screen
        options={{
          headerSearchBarOptions: {
            placeholder: "Find a model",
            hideWhenScrolling: false,
            autoCapitalize: "none",
            onChangeText: (event) => setQuery(event.nativeEvent.text),
          },
        }}
      />
      <ModelList
        projectId=""
        sessionId={pilotConversationId(pilotId)}
        query={query}
        onOpenOptions={() =>
          router.push({ pathname: "/pilot-profile/[pilotId]/model-options", params: { pilotId } })
        }
      />
    </>
  );
}
