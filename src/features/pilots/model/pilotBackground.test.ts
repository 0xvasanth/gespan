// @vitest-environment happy-dom
import { afterEach, expect, it } from "vitest";
import { CHAT_BACKGROUND_OPACITY_DEFAULT } from "../../settings/model/appearance";
import { saveProjectChatBackgroundSettings } from "../../projects/model/projectChatBackground";
import { pilotBackgroundKey, pilotChatBackground } from "./pilotBackground";

afterEach(() => localStorage.clear());

it("always shows a Pilot's background as a dimmed Haze", () => {
  expect(pilotChatBackground("a")).toBeNull();
  saveProjectChatBackgroundSettings(pilotBackgroundKey("a"), {
    path: "/bg/pilot.png",
    emptyOpacity: 0.6,
    sessionOpacity: 0.5,
    scope: "empty",
    effect: "dither",
  });
  expect(pilotChatBackground("a")).toEqual({
    path: "/bg/pilot.png",
    emptyOpacity: CHAT_BACKGROUND_OPACITY_DEFAULT,
    sessionOpacity: CHAT_BACKGROUND_OPACITY_DEFAULT,
    scope: "all",
    effect: "gradient-blur",
  });
});
