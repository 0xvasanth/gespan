import type { ReleaseFiles } from "../scripts/release.mjs";

export type Recommendation =
  | { os: "mac"; label: string; detail: string; href: string; alternate: { label: string; href: string } }
  | { os: "windows" | "linux"; label: string; detail: string; href: string }
  | { os: "phone" }
  | { os: "unknown" };

/**
 * The download to offer first, from the browser's user agent. A Mac's chip
 * can't be read reliably, so Macs get Apple Silicon with the Intel build one
 * click away. Phones and tablets get no download: Gespan is a desktop app.
 */
export function recommendDownload(userAgent: string, files: ReleaseFiles, maxTouchPoints = 0): Recommendation {
  const ua = userAgent.toLowerCase();
  // iPadOS reports a Mac user agent; touch points give it away.
  if (/iphone|ipad|ipod|android|mobile/.test(ua) || (/macintosh/.test(ua) && maxTouchPoints > 1)) return { os: "phone" };
  if (/macintosh|mac os x/.test(ua))
    return { os: "mac", label: "For your Mac", detail: "macOS, Apple Silicon · signed and notarized", href: files.macArm, alternate: { label: "Intel Mac? Get the Intel .dmg", href: files.macIntel } };
  if (/windows/.test(ua)) return { os: "windows", label: "For your PC", detail: "Windows, x64 installer", href: files.windows };
  if (/linux|x11/.test(ua)) return { os: "linux", label: "For your Linux machine", detail: "x64 AppImage", href: files.appImage };
  return { os: "unknown" };
}
