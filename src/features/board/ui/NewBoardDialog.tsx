import { useEffect, useRef, useState } from "react";
import { Modal } from "../../../shared/ui/Modal";
import { listTemplates, type TemplateInfo } from "../model/board";
import { errorText } from "./boardFormat";
import { orderTemplates } from "./settings/TemplatesSection";
import {
  FIELD,
  RadioGroup,
  SETTINGS_BUTTON,
} from "./settings/settingsControls";

const BLANK: TemplateInfo = { id: "blank", name: "Blank", builtIn: true };

/** New board: a name and a template (Blank unless another is picked). */
export function NewBoardDialog({
  onCreate,
  onClose,
}: {
  onCreate: (name: string, templateId: string) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [missing, setMissing] = useState(false);
  const [templates, setTemplates] = useState<TemplateInfo[]>([BLANK]);
  const [templateId, setTemplateId] = useState(BLANK.id);
  const [note, setNote] = useState<string | null>(null);
  const field = useRef<HTMLInputElement>(null);

  useEffect(() => {
    field.current?.focus();
    let live = true;
    listTemplates().then(
      (list) => {
        if (live && list.length) setTemplates(orderTemplates(list));
      },
      (cause) => {
        // Blank still works without the list.
        if (live) setNote(`Couldn't load templates: ${errorText(cause)}`);
      },
    );
    return () => {
      live = false;
    };
  }, []);

  const submit = () => {
    const trimmed = name.trim();
    if (!trimmed) {
      setMissing(true);
      field.current?.focus();
      return;
    }
    onCreate(trimmed, templateId);
  };

  return (
    <Modal title="New board" size="sm" hideClose onClose={onClose}>
      <form
        className="flex flex-col gap-3 px-4 pb-4 pt-2"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <div className="flex flex-col gap-1">
          <input
            ref={field}
            aria-label="Board name"
            aria-invalid={missing || undefined}
            placeholder="Board name"
            value={name}
            onChange={(event) => {
              setName(event.target.value);
              setMissing(false);
            }}
            className={`h-9 ${FIELD}`}
          />
          {missing ? (
            <span role="alert" className="text-[12px] text-red-300/90">
              Add a name
            </span>
          ) : null}
        </div>
        <div className="flex flex-col gap-1.5">
          <span className="text-[12px] text-content/60">Template</span>
          <RadioGroup
            label="Template"
            className="flex flex-col gap-1"
            value={templateId}
            onChange={setTemplateId}
            options={templates.map((template) => ({
              value: template.id,
              label: template.name,
              content: (
                <>
                  <span className="min-w-0 flex-1 truncate">{template.name}</span>
                  {template.builtIn ? null : (
                    <span className="text-[11px] text-content/45">Saved</span>
                  )}
                </>
              ),
              className: `flex h-8 w-full items-center gap-2 rounded-md border px-2.5 text-left text-[12px] outline-none focus-visible:ring-2 focus-visible:ring-accent/60 ${
                templateId === template.id
                  ? "border-accent/60 bg-content/10 text-content"
                  : "border-content/15 text-content/75 hover:bg-content/10"
              }`,
            }))}
          />
          {note ? <span className="text-[11px] text-content/50">{note}</span> : null}
        </div>
        <div className="flex justify-end gap-2">
          <button type="button" className={SETTINGS_BUTTON} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className={SETTINGS_BUTTON}>
            Create board
          </button>
        </div>
      </form>
    </Modal>
  );
}
