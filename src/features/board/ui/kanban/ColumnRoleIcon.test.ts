// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type { ColumnRole } from "../../model/board";
import { ColumnRoleIcon } from "./ColumnRoleIcon";

let host: HTMLDivElement | null = null;

function render(role: ColumnRole | null): SVGElement {
  host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  act(() => root.render(createElement(ColumnRoleIcon, { role })));
  return host.querySelector("svg")!;
}

afterEach(() => {
  host?.remove();
  host = null;
});

describe("ColumnRoleIcon", () => {
  it("draws a cancelled column as a muted, struck-through ring", () => {
    const icon = render("cancelled");
    expect(icon.dataset.role).toBe("cancelled");
    expect(icon.getAttribute("class")).toContain("text-content/45");
    expect(icon.getAttribute("class")).not.toContain("undefined");
    expect(icon.querySelector("[data-mark='strike']")).not.toBeNull();
  });

  it("keeps the strike off every other role", () => {
    for (const role of ["todo", "doing", "review", "done", null] as const) {
      expect(render(role).querySelector("[data-mark='strike']")).toBeNull();
    }
  });
});
