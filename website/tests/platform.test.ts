import { describe, expect, test } from "vitest";
import { recommendDownload } from "../src/platform";

const files = { macArm: "mac-arm.dmg", macIntel: "mac-x64.dmg", windows: "setup.exe", appImage: "app.AppImage", deb: "a.deb", rpm: "a.rpm" };
const UA = {
  mac: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Safari/605.1.15",
  windows: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36",
  linux: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36",
  iphone: "Mozilla/5.0 (iPhone; CPU iPhone OS 19_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148",
  android: "Mozilla/5.0 (Linux; Android 16; Pixel 10) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Mobile Safari/537.36",
};

describe("recommendDownload", () => {
  test("a Mac gets Apple Silicon with the Intel build one click away", () => {
    const pick = recommendDownload(UA.mac, files);
    expect(pick).toMatchObject({ os: "mac", href: "mac-arm.dmg", alternate: { href: "mac-x64.dmg" } });
  });

  test("Windows gets the installer and Linux the AppImage", () => {
    expect(recommendDownload(UA.windows, files)).toMatchObject({ os: "windows", href: "setup.exe" });
    expect(recommendDownload(UA.linux, files)).toMatchObject({ os: "linux", href: "app.AppImage" });
  });

  test("phones get no download, Android included, though its agent says Linux", () => {
    expect(recommendDownload(UA.iphone, files).os).toBe("phone");
    expect(recommendDownload(UA.android, files).os).toBe("phone");
  });

  test("an iPad, which reports a Mac agent, is told apart by touch", () => {
    expect(recommendDownload(UA.mac, files, 5).os).toBe("phone");
    expect(recommendDownload(UA.mac, files, 0).os).toBe("mac");
  });

  test("an agent it can't read offers the full list instead of guessing", () => {
    expect(recommendDownload("curl/8.0", files).os).toBe("unknown");
  });
});
