// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { LinkSessionWorkItemDialog } from "./LinkSessionWorkItemDialog";
import type { LinkedWorkItem } from "../model/session";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const LINEAR = {
  tracker: "linear",
  key: "ENG-7",
  url: "https://linear.app/acme/issue/ENG-7",
  title: null,
  status: null,
  statusGroup: null,
  fetchedAt: null,
};

let container: HTMLDivElement;
let root: Root;
let resolve: (input: string) => unknown;
let onSave: ReturnType<
  typeof vi.fn<(item: LinkedWorkItem | undefined) => void>
>;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  onSave = vi.fn();
  resolve = () => ({ kind: "one", ticket: LINEAR });
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "ticket_resolve")
      return resolve((args as { input: string }).input);
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(async () => {
  await act(async () => root.unmount());
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function open(initial?: LinkedWorkItem) {
  await act(async () =>
    root.render(
      createElement(LinkSessionWorkItemDialog, {
        initial,
        sessionTitle: "Fix login",
        onSave,
        onClose: () => undefined,
      }),
    ),
  );
}

async function type(value: string) {
  const field = document.querySelector<HTMLInputElement>("input")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function submit() {
  await act(async () => {
    document
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
}

function resolveCalls() {
  return vi.mocked(invoke).mock.calls.filter(([c]) => c === "ticket_resolve");
}

it("keeps GitHub links on the existing path without asking the tracker commands", async () => {
  await open();
  await type("https://github.com/acme/app/pull/12");
  await submit();
  expect(onSave).toHaveBeenCalledWith({
    kind: "pr",
    repo: "acme/app",
    number: 12,
    url: "https://github.com/acme/app/pull/12",
  });
  expect(resolveCalls()).toHaveLength(0);
});

it("links a ticket URL as soon as it resolves", async () => {
  await open();
  await type("https://linear.app/acme/issue/ENG-7");
  await submit();
  expect(resolveCalls()[0][1]).toEqual({
    input: "https://linear.app/acme/issue/ENG-7",
  });
  expect(onSave).toHaveBeenCalledWith({ kind: "ticket", ticket: LINEAR });
});

it("asks which tracker owns a bare key when several are connected", async () => {
  resolve = () => ({
    kind: "ambiguous",
    key: "ENG-7",
    trackers: ["jira", "linear"],
  });
  await open();
  await type("ENG-7");
  await submit();
  expect(onSave).not.toHaveBeenCalled();
  const choices = [
    ...document.querySelectorAll<HTMLButtonElement>("[data-tracker-choice]"),
  ];
  expect(choices.map((b) => b.textContent)).toEqual(["Jira", "Linear"]);
  await act(async () => choices[1].click());
  expect(onSave).toHaveBeenCalledWith({
    kind: "ticket",
    ticket: {
      tracker: "linear",
      key: "ENG-7",
      url: null,
      title: null,
      status: null,
      statusGroup: null,
      fetchedAt: null,
    },
  });
});

it("shows the reason inline and saves nothing when the input does not resolve", async () => {
  resolve = () => ({ kind: "error", message: "Connect Linear in Settings" });
  await open();
  await type("https://linear.app/acme/issue/ENG-7");
  await submit();
  expect(document.querySelector('[role="alert"]')?.textContent).toBe(
    "Connect Linear in Settings",
  );
  expect(onSave).not.toHaveBeenCalled();
});

it("shows an error when the resolve command itself fails", async () => {
  resolve = () => {
    throw "boom";
  };
  await open();
  await type("ENG-7");
  await submit();
  expect(document.querySelector('[role="alert"]')?.textContent).toBe("boom");
  expect(onSave).not.toHaveBeenCalled();
});

it("ignores empty input", async () => {
  await open();
  await type("   ");
  await submit();
  expect(resolveCalls()).toHaveLength(0);
  expect(onSave).not.toHaveBeenCalled();
});

it("prefills an existing ticket link and can remove it", async () => {
  await open({ kind: "ticket", ticket: LINEAR as never });
  expect(document.querySelector<HTMLInputElement>("input")!.value).toBe(
    "https://linear.app/acme/issue/ENG-7",
  );
  const remove = [...document.querySelectorAll("button")].find(
    (b) => b.textContent === "Remove link",
  )!;
  await act(async () => remove.click());
  expect(onSave).toHaveBeenCalledWith(undefined);
});
