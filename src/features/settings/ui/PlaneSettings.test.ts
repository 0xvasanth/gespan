// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { PlaneSettings } from "./PlaneSettings";
import { TICKET_CONNECTIONS_EVENT } from "../../sessions/model/ticketLinks";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

let container: HTMLDivElement;
let root: Root;
let connectionEvents = 0;
const onConnections = () => {
  connectionEvents += 1;
};

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  connectionEvents = 0;
  window.addEventListener(TICKET_CONNECTIONS_EVENT, onConnections);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "plane_status")
      return { connected: false, baseUrl: "", workspace: "" };
    if (command === "plane_set_config") {
      const config = args as {
        baseUrl: string;
        workspace: string;
        apiKey: string;
      };
      return {
        connected: Boolean(config.apiKey),
        baseUrl: config.apiKey ? config.baseUrl : "",
        workspace: config.apiKey ? config.workspace : "",
      };
    }
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(async () => {
  window.removeEventListener(TICKET_CONNECTIONS_EVENT, onConnections);
  await act(async () => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

async function input(label: string, value: string) {
  const field = container.querySelector<HTMLInputElement>(
    `input[aria-label="${label}"]`,
  )!;
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
    container
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
}

function button(text: string) {
  return [...container.querySelectorAll("button")].find(
    (b) => b.textContent === text,
  );
}

it("connects Plane with base URL, workspace and key, then disconnects", async () => {
  await act(async () => root.render(createElement(PlaneSettings)));
  expect(button("Connect")!.disabled).toBe(true);
  await input("Plane base URL", " https://plane.acme.dev ");
  await input("Plane workspace", "acme");
  await input("Plane API key", "plane_api_secret");
  await submit();
  expect(invoke).toHaveBeenCalledWith("plane_set_config", {
    baseUrl: "https://plane.acme.dev",
    workspace: "acme",
    apiKey: "plane_api_secret",
  });
  expect(container.textContent).toContain("https://plane.acme.dev");
  expect(container.textContent).toContain("acme");
  expect(container.querySelector('input[type="password"]')).toBeNull();
  expect(connectionEvents).toBe(1);

  await act(async () => button("Disconnect")!.click());
  expect(invoke).toHaveBeenCalledWith("plane_set_config", {
    baseUrl: "",
    workspace: "",
    apiKey: "",
  });
  expect(container.querySelector("form")).not.toBeNull();
  expect(
    container.querySelector<HTMLInputElement>('input[type="password"]')!.value,
  ).toBe("");
  expect(connectionEvents).toBe(2);
});

it("shows an existing connection without asking for the key again", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "plane_status")
      return {
        connected: true,
        baseUrl: "https://api.plane.so",
        workspace: "eng",
      };
    throw new Error(`Unexpected command: ${command}`);
  });
  await act(async () => root.render(createElement(PlaneSettings)));
  expect(container.textContent).toContain("https://api.plane.so");
  expect(container.textContent).toContain("eng");
  expect(container.querySelector("form")).toBeNull();
});

it("keeps the form and shows the error when Plane rejects the key", async () => {
  await act(async () => root.render(createElement(PlaneSettings)));
  await input("Plane base URL", "https://api.plane.so");
  await input("Plane workspace", "acme");
  await input("Plane API key", "bad");
  vi.mocked(invoke).mockRejectedValueOnce("Plane API key is invalid");
  await submit();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "invalid",
  );
  expect(container.querySelector("form")).not.toBeNull();
  expect(connectionEvents).toBe(0);
});
