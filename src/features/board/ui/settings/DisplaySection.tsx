import { useEffect, useRef } from "react";
import type { BoardField, BoardSettings, DisplayOptions } from "../../model/board";
import { parseDisplay, serializeDisplay } from "../../model/display";
import { DisplayEditor } from "./DisplayEditor";
import { SettingsSection } from "./settingsControls";

/** The board's default display. A saved view may override it from the toolbar. */
export function DisplaySection({
  settings,
  fields,
  onPatch,
}: {
  settings: BoardSettings | null;
  fields: readonly BoardField[];
  onPatch: (patch: Partial<BoardSettings>, what: string) => Promise<unknown>;
}) {
  const display = parseDisplay(settings?.displayJson ?? null);
  // Quick toggles build on the latest display, saved or not, and write in order.
  const latest = useRef(display);
  const pending = useRef(0);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const json = settings?.displayJson ?? null;
  useEffect(() => {
    if (pending.current === 0) latest.current = parseDisplay(json);
  }, [json]);
  const save = (next: DisplayOptions) => {
    latest.current = next;
    pending.current += 1;
    queue.current = queue.current
      .then(() => onPatch({ displayJson: serializeDisplay(next) }, "save the display"))
      .finally(() => {
        pending.current -= 1;
      });
  };
  return (
    <SettingsSection
      title="Display"
      hint="How cards look by default. A saved view can override this from the Display menu."
    >
      <DisplayEditor
        display={display}
        fields={fields}
        onChange={(next) => save(next)}
        base={() => latest.current}
      />
    </SettingsSection>
  );
}
