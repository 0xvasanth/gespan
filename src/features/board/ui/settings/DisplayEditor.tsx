import type { AgentField, BoardField, DisplayOptions } from "../../model/board";
import { AGENT_FIELDS, FACE_TOGGLES } from "../../model/display";
import { FIELD, RadioGroup } from "./settingsControls";

const toggled = <T,>(list: readonly T[], item: T): T[] =>
  list.includes(item) ? list.filter((held) => held !== item) : [...list, item];

/**
 * The controls of a display: which parts a card shows, its fields, density
 * and swimlanes. Used for the board default and, in the toolbar, for a view.
 */
export function DisplayEditor({
  display,
  fields,
  onChange: change,
  base,
  disabled = false,
}: {
  display: DisplayOptions;
  fields: readonly BoardField[];
  onChange: (next: DisplayOptions) => void;
  /** The display an edit builds on, when that is newer than `display` (writes in flight). */
  base?: () => DisplayOptions;
  /** Shows the display but takes no edits. */
  disabled?: boolean;
}) {
  const onChange = (next: DisplayOptions) => {
    if (!disabled) change(next);
  };
  const current = () => base?.() ?? display;
  const setFace = (patch: Partial<DisplayOptions["face"]>) =>
    onChange({ ...current(), face: { ...current().face, ...patch } });
  // A listed field that was deleted stays out of the checklist and the face.
  const known = new Set(fields.map((field) => field.id));
  const swimlanes = [
    ["none", "None"],
    ["priority", "Priority"],
    ["label", "Label"],
    ["project", "Project"],
    ...fields
      .filter((field) => field.kind === "select")
      .map((field): [string, string] => [`field:${field.id}`, field.name]),
  ];
  const lane = swimlanes.some(([value]) => value === display.swimlane)
    ? display.swimlane
    : "none";
  return (
    <fieldset
      disabled={disabled}
      className={`flex min-w-0 flex-col gap-3 text-[12px] text-content/80 ${disabled ? "opacity-50" : ""}`}
    >
      <fieldset className="flex flex-wrap gap-x-4 gap-y-1.5">
        <legend className="sr-only">Show on cards</legend>
        {FACE_TOGGLES.map(([key, label]) => (
          <label key={key} className="flex items-center gap-1.5">
            <input
              type="checkbox"
              disabled={disabled}
              aria-label={`Show ${label}`}
              checked={display.face[key]}
              onChange={(event) => setFace({ [key]: event.target.checked })}
            />
            {label}
          </label>
        ))}
      </fieldset>
      {fields.length ? (
        <fieldset className="flex flex-wrap gap-x-4 gap-y-1.5">
          <legend className="pb-1 text-content/55">Custom fields on cards</legend>
          {fields.map((field) => (
            <label key={field.id} className="flex items-center gap-1.5">
              <input
                type="checkbox"
              disabled={disabled}
                aria-label={`Show field ${field.name}`}
                checked={display.face.fields.includes(field.id)}
                onChange={() =>
                  setFace({
                    fields: toggled(
                      current().face.fields.filter((id) => known.has(id)),
                      field.id,
                    ),
                  })
                }
              />
              {field.name}
            </label>
          ))}
        </fieldset>
      ) : null}
      <fieldset className="flex flex-wrap gap-x-4 gap-y-1.5">
        <legend className="pb-1 text-content/55">Agent details</legend>
        {AGENT_FIELDS.map(([key, label]) => (
          <label key={key} className="flex items-center gap-1.5">
            <input
              type="checkbox"
              disabled={disabled}
              aria-label={`Show agent ${label}`}
              checked={display.face.agentFields.includes(key)}
              onChange={() =>
                setFace({
                  agentFields: toggled<AgentField>(current().face.agentFields, key),
                })
              }
            />
            {label}
          </label>
        ))}
      </fieldset>
      <div className="flex items-center gap-3">
        <span className="w-16 shrink-0">Density</span>
        <RadioGroup
          label="Density"
          value={display.density}
          onChange={(density) =>
            onChange({ ...current(), density: density as DisplayOptions["density"] })
          }
          options={(["comfortable", "compact"] as const).map((density) => ({
            value: density,
            label: density === "compact" ? "Compact" : "Comfortable",
            content: density === "compact" ? "Compact" : "Comfortable",
            className: `h-7 rounded-md border px-2 text-[12px] outline-none focus-visible:ring-2 focus-visible:ring-accent/60 ${
              display.density === density
                ? "border-accent/60 bg-content/10 text-content"
                : "border-content/15 text-content/70 hover:bg-content/10"
            }`,
          }))}
        />
      </div>
      <label className="flex items-center gap-3">
        <span className="w-16 shrink-0">Swimlanes</span>
        <select
          aria-label="Swimlanes"
          disabled={disabled}
          value={lane}
          onChange={(event) => onChange({ ...current(), swimlane: event.target.value })}
          className={`${FIELD} w-44`}
        >
          {swimlanes.map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
      </label>
    </fieldset>
  );
}
