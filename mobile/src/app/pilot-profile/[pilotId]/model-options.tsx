import { useLocalSearchParams } from "expo-router";

import { ModelOptions } from "@/features/models/ModelPicker";
import { pilotConversationId } from "@/features/workspace/WorkspaceProvider";

// The Pilot's model options and access, pushed from its model list.
export default function PilotModelOptionsScreen() {
  const { pilotId } = useLocalSearchParams<{ pilotId: string }>();
  return <ModelOptions projectId="" sessionId={pilotConversationId(pilotId)} />;
}
