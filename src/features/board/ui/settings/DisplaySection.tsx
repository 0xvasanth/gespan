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
  onPatch: (patch: Partial<BoardSettings>, what: string) => Promise<boolean>;
}) {
  const display = parseDisplay(settings?.displayJson ?? null);
  // Quick toggles build on the latest display, saved or not, and write in order.
  const latest = useRef(display);
  const pending = useRef(0);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const json = settings?.displayJson ?? null;
  const stored = useRef(json);
  stored.current = json;
  useEffect(() => {
    if (pending.current === 0) latest.current = parseDisplay(json);
  }, [json]);
  const save = (next: DisplayOptions) => {
    latest.current = next;
    pending.current += 1;
    queue.current = queue.current
      .then(() =>
        onPatch({ displayJson: serializeDisplay(next) }, "save the display"),
      )
      .then((saved) => {
        pending.current -= 1;
        // A save that failed leaves what is stored; edits stop building on it.
        if (!saved && pending.current === 0) {
          latest.current = parseDisplay(stored.current);
        }
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
