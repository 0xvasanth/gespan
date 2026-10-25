// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { insertDictation } from "./insertDictation";

function field(value: string, start = value.length, end = start) {
  const el = document.createElement("textarea");
  document.body.append(el);
  el.value = value;
  el.setSelectionRange(start, end);
  return el;
}

afterEach(() => {
  document.body.replaceChildren();
});

describe("insertDictation", () => {
  it("inserts_at_caret_with_leading_space", () => {
    const el = field("rename the", 10);
    insertDictation(el, "helper");
    expect(el.value).toBe("rename the helper");
    expect(el.selectionStart).toBe(17);
    expect(el.selectionEnd).toBe(17);

    // Mid-word caret: still separated from the word before it.
    const mid = field("fix bug now", 7);
    insertDictation(mid, "in parser");
    expect(mid.value).toBe("fix bug in parser now");
    expect(mid.selectionStart).toBe(17);
  });

  it("no_space_after_whitespace_or_before_punctuation", () => {
    const empty = field("");
    insertDictation(empty, "Hello");
    expect(empty.value).toBe("Hello");

    const afterSpace = field("say ");
    insertDictation(afterSpace, "hi");
    expect(afterSpace.value).toBe("say hi");

    const afterNewline = field("line one\n");
    insertDictation(afterNewline, "line two");
    expect(afterNewline.value).toBe("line one\nline two");

    for (const mark of [".", ",", ";", ":", "!", "?", ")"]) {
      const el = field("done");
      insertDictation(el, `${mark} next`);
      expect(el.value).toBe(`done${mark} next`);
    }

    // Other punctuation-led text still gets the space.
    const quote = field("say");
    insertDictation(quote, '"hi"');
    expect(quote.value).toBe('say "hi"');
  });

  it("replaces_selection", () => {
    const el = field("open main.ts now", 5, 12);
    insertDictation(el, "lib.rs");
    expect(el.value).toBe("open lib.rs now");
    expect(el.selectionStart).toBe(11);
    expect(el.selectionEnd).toBe(11);

    // A whole-draft selection is replaced without a stray leading space.
    const all = field("old draft", 0, 9);
    insertDictation(all, "New draft");
    expect(all.value).toBe("New draft");
  });

  it("fires_input_event", () => {
    const el = field("a");
    const seen: string[] = [];
    const onInput = vi.fn((event: Event) => {
      seen.push((event.target as HTMLTextAreaElement).value);
      expect(event.bubbles).toBe(true);
    });
    document.body.addEventListener("input", onInput);
    insertDictation(el, "b");
    document.body.removeEventListener("input", onInput);
    expect(onInput).toHaveBeenCalledTimes(1);
    // The mirror sees the inserted text, as React's onInput would.
    expect(seen).toEqual(["a b"]);
  });

  it("ignores_empty_text", () => {
    const el = field("keep", 2);
    const onInput = vi.fn();
    el.addEventListener("input", onInput);
    insertDictation(el, "   ");
    expect(el.value).toBe("keep");
    expect(el.selectionStart).toBe(2);
    expect(onInput).not.toHaveBeenCalled();
  });

  it("final_inserts_at_caret_at_stop_time", () => {
    // Dictation starts with the caret at the end of "fix ".
    const el = field("fix ");
    // While listening the user types, then moves the caret back after "fix".
    el.value = "fix the loader";
    el.setSelectionRange(3, 3);
    insertDictation(el, "please");
    // The final lands where the caret is now; the typed text is kept.
    expect(el.value).toBe("fix please the loader");
    expect(el.selectionStart).toBe(10);

    const later = field("fix ");
    later.value = "fix the loader";
    later.setSelectionRange(14, 14);
    insertDictation(later, "in main.rs");
    expect(later.value).toBe("fix the loader in main.rs");
  });
});
