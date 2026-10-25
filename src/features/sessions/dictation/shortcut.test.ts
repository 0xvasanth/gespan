// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from "vitest";
import { IS_MAC } from "../../../platform/tauri/platform";
import { saveKeybindingOverride } from "../../settings/model/settings";
import { DICTATE_COMMAND, isDictationShortcut } from "./shortcut";

function chord(over: Partial<KeyboardEvent> = {}) {
  return {
    key: "M",
    code: "KeyM",
    metaKey: IS_MAC,
    ctrlKey: !IS_MAC,
    altKey: false,
    shiftKey: true,
    ...over,
  };
}

beforeEach(() => localStorage.clear());

describe("isDictationShortcut", () => {
  it("matches_platform_mod_shift_m_only", () => {
    expect(isDictationShortcut(chord())).toBe(true);
    expect(isDictationShortcut(chord({ shiftKey: false, key: "m" }))).toBe(
      false,
    );
    expect(isDictationShortcut(chord({ altKey: true }))).toBe(false);
    // The other platform's modifier is not the default.
    expect(
      isDictationShortcut(chord({ metaKey: !IS_MAC, ctrlKey: IS_MAC })),
    ).toBe(false);
    expect(isDictationShortcut(chord({ key: "N", code: "KeyN" }))).toBe(false);
  });

  it("honours_a_rebound_or_disabled_shortcut", () => {
    saveKeybindingOverride(DICTATE_COMMAND, { shortcut: "Option+Shift+KeyD" });
    expect(isDictationShortcut(chord())).toBe(false);
    expect(
      isDictationShortcut(
        chord({
          key: "D",
          code: "KeyD",
          metaKey: false,
          ctrlKey: false,
          altKey: true,
        }),
      ),
    ).toBe(true);

    saveKeybindingOverride(DICTATE_COMMAND, { disabled: true });
    expect(isDictationShortcut(chord())).toBe(false);
  });

  it("reserves_the_default_chord_against_other_commands", () => {
    const chordName = `${IS_MAC ? "Command" : "Control"}+Shift+KeyM`;
    expect(() =>
      saveKeybindingOverride("App: Search", { shortcut: chordName }),
    ).toThrow("Already used by Composer: Dictate");
  });
});
