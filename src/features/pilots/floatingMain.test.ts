// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";

const native = vi.hoisted(() => ({ invoke: vi.fn(), setTheme: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: native.invoke }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme: native.setTheme }),
}));
vi.mock("../../platform/tauri/platform", () => ({
  IS_MAC: true,
  IS_LINUX: false,
}));
vi.mock("../../platform/tauri/fs", () => ({ homeDir: async () => "/tmp" }));
vi.mock("react-dom/client", () => ({
  default: { createRoot: () => ({ render() {} }) },
}));
vi.mock("./ui/FloatingPilotChat", () => ({ FloatingPilotChat: () => null }));

afterEach(() => {
  document.getElementById("test-floating-theme")?.remove();
  document.body.innerHTML = "";
  document.documentElement.className = "";
  document.documentElement.style.cssText = "";
  localStorage.clear();
  vi.restoreAllMocks();
});

it("uses the main app's native glass settings and follows changes from other windows", async () => {
  vi.resetModules();
  document.body.innerHTML = '<div id="root"></div>';
  // Vitest omits the CSS entry's rules; provide its light-scheme token.
  const theme = document.createElement("style");
  theme.id = "test-floating-theme";
  theme.textContent = "html.theme-light { --background-lightness: 97%; }";
  document.head.append(theme);
  document.documentElement.style.setProperty(
    "--motion-feedback-duration",
    "120ms",
  );
  localStorage.setItem("gespan.colorScheme", "dark");
  localStorage.setItem("gespan.sidebarOpacity", "0.5");
  localStorage.setItem("gespan.sidebarBlur", "32");
  localStorage.setItem("gespan.bodyGlass", "1");
  native.invoke.mockResolvedValue(true);
  native.setTheme.mockResolvedValue(undefined);
  const listen = vi.spyOn(window, "addEventListener");
  await import("./floatingMain");
  expect(native.invoke).toHaveBeenCalledWith("set_window_background_blur", {
    radius: 32,
  });
  expect(native.invoke).toHaveBeenCalledWith("set_window_glass_enabled", {
    enabled: true,
    background: { r: 23, g: 23, b: 23 },
    opacity: 0.7,
  });
  await vi.waitFor(() =>
    expect(
      document.documentElement.classList.contains("has-native-glass-tint"),
    ).toBe(true),
  );
  expect(document.documentElement.classList.contains("glass-body")).toBe(true);

  localStorage.setItem("gespan.sidebarOpacity", "0.6");
  window.dispatchEvent(
    new StorageEvent("storage", { key: "gespan.sidebarOpacity" }),
  );
  expect(native.invoke).toHaveBeenLastCalledWith("set_window_glass_enabled", {
    enabled: true,
    background: { r: 23, g: 23, b: 23 },
    opacity: 0.76,
  });

  localStorage.setItem("gespan.colorScheme", "light");
  window.dispatchEvent(
    new StorageEvent("storage", { key: "gespan.colorScheme" }),
  );
  expect(
    document.documentElement.classList.contains("has-native-glass-tint"),
  ).toBe(false);
  await vi.waitFor(() =>
    expect(native.invoke).toHaveBeenLastCalledWith("set_window_glass_enabled", {
      enabled: false,
      background: { r: 247, g: 247, b: 247 },
      opacity: 0.76,
    }),
  );
  expect(native.setTheme).toHaveBeenLastCalledWith("light");

  for (const [event, handler] of listen.mock.calls) {
    if (event === "storage") window.removeEventListener(event, handler);
  }
});
