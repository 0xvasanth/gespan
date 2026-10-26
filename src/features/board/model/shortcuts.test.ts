// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { focusedCardId, isEditableTarget, shortcutFor } from "./shortcuts";

const el = (html: string) => {
  const host = document.createElement("div");
  host.innerHTML = html;
  document.body.append(host);
  return host;
};

describe("isEditableTarget", () => {
  it("is true for input, textarea and select", () => {
    for (const tag of ["input", "textarea", "select"]) {
      expect(isEditableTarget(document.createElement(tag))).toBe(true);
    }
  });

  it("is true for contenteditable and anything inside it", () => {
    const host = el('<div contenteditable="true"><p id="p">x</p></div>');
    expect(isEditableTarget(host.querySelector("#p"))).toBe(true);
    expect(isEditableTarget(host.firstElementChild)).toBe(true);
    host.remove();
  });

  it("is true inside a CodeMirror editor and a textbox role", () => {
    const host = el(
      '<div class="cm-editor"><div class="cm-line" id="l"></div></div><div role="textbox" id="t"></div>',
    );
    expect(isEditableTarget(host.querySelector("#l"))).toBe(true);
    expect(isEditableTarget(host.querySelector("#t"))).toBe(true);
    host.remove();
  });

  it("is false for buttons, plain elements, contenteditable=false and null", () => {
    const host = el('<button id="b"></button><div contenteditable="false" id="f"></div>');
    expect(isEditableTarget(host.querySelector("#b"))).toBe(false);
    expect(isEditableTarget(host.querySelector("#f"))).toBe(false);
    expect(isEditableTarget(document.body)).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
    host.remove();
  });
});

describe("shortcutFor", () => {
  const base = { metaKey: false, ctrlKey: false, altKey: false };
  it("maps the board keys", () => {
    expect(shortcutFor({ ...base, key: "n" })).toBe("new");
    expect(shortcutFor({ ...base, key: "E" })).toBe("open");
    expect(shortcutFor({ ...base, key: "/" })).toBe("search");
    expect(shortcutFor({ ...base, key: "Delete" })).toBe("delete");
    expect(shortcutFor({ ...base, key: "Backspace" })).toBe("delete");
  });
  it("ignores modifiers, composition, repeats and handled events", () => {
    expect(shortcutFor({ ...base, key: "n", metaKey: true })).toBeNull();
    expect(shortcutFor({ ...base, key: "n", ctrlKey: true })).toBeNull();
    expect(shortcutFor({ ...base, key: "e", altKey: true })).toBeNull();
    expect(shortcutFor({ ...base, key: "n", isComposing: true })).toBeNull();
    expect(shortcutFor({ ...base, key: "Delete", repeat: true })).toBeNull();
    expect(shortcutFor({ ...base, key: "n", defaultPrevented: true })).toBeNull();
    expect(shortcutFor({ ...base, key: "x" })).toBeNull();
  });
});

describe("focusedCardId", () => {
  it("finds the card of a kanban wrapper, list row and table row", () => {
    const host = el(
      '<div data-kanban-card="k"><span id="in"></span><button id="btn"></button></div><div data-card-id="l" id="l"></div><div data-table-row="t" id="t"></div>',
    );
    expect(focusedCardId(host.querySelector("#in"))).toBe("k");
    expect(focusedCardId(host.querySelector("#l"))).toBe("l");
    expect(focusedCardId(host.querySelector("#t"))).toBe("t");
    host.remove();
  });
  it("is null for a control inside a card and for non-cards", () => {
    const host = el('<div data-kanban-card="k"><button id="btn"></button></div>');
    expect(focusedCardId(host.querySelector("#btn"))).toBeNull();
    expect(focusedCardId(document.body)).toBeNull();
    host.remove();
  });
});
