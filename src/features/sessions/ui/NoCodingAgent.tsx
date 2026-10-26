import { useSyncExternalStore } from "react";
import {
  getHarnessAvailabilitySnapshot,
  subscribeHarnessAvailability,
} from "../../../integrations/harness/core/availabilityState";
import {
  NO_CODING_AGENT_HINT,
  noCodingAgentDetected,
  openAgentClisSettings,
} from "../model/noCodingAgent";

/** Re-reads on every probe, so the composer re-enables once a CLI appears. */
export function useNoCodingAgentDetected(): boolean {
  useSyncExternalStore(
    subscribeHarnessAvailability,
    getHarnessAvailabilitySnapshot,
    getHarnessAvailabilitySnapshot,
  );
  return noCodingAgentDetected();
}

/** Sits beside the disabled model picker; opens Settings → Agent CLIs. */
export function NoCodingAgentSettingsButton({
  onOpen = openAgentClisSettings,
}: {
  onOpen?: () => void;
}) {
  return (
    <button
      type="button"
      data-no-coding-agent="settings"
      title={NO_CODING_AGENT_HINT}
      onMouseDown={(event) => event.preventDefault()}
      onClick={onOpen}
      className="flex h-6.5 shrink-0 items-center rounded-md px-1.5 text-[12px] text-amber-400 hover:bg-selection-hover hover:text-amber-300"
    >
      Open Settings
    </button>
  );
}
