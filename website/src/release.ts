import type { ChangelogRelease, Release } from "../scripts/release.mjs";

declare const __GESPAN_RELEASE__: Release;
declare const __GESPAN_CHANGELOG__: ChangelogRelease[];

/** The current release, read from README.md at build time. */
export const release: Release = __GESPAN_RELEASE__;
/** The newest releases in CHANGELOG.md, read at build time. */
export const changelog: ChangelogRelease[] = __GESPAN_CHANGELOG__;

/** A release's headline: its first `count` titled entries, "A, B and C". */
export function highlights(entry: ChangelogRelease | undefined, count = 3): string {
  const titles = (entry?.sections ?? []).flatMap((section) => section.entries).map((e) => e.title).filter((t): t is string => Boolean(t)).slice(0, count);
  if (titles.length < 2) return titles.join("");
  return `${titles.slice(0, -1).join(", ")} and ${titles[titles.length - 1]}`;
}

/** "2026-11-14" → "14 November 2026". */
export function formatDate(iso: string | null): string {
  if (!iso) return "";
  const [y, m, d] = iso.split("-").map(Number);
  const months = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
  return `${d} ${months[m - 1]} ${y}`;
}
