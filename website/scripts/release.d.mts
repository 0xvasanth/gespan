export interface ChangelogEntry {
  title: string | null;
  body: string;
}
export interface ChangelogSection {
  kind: string;
  entries: ChangelogEntry[];
}
export interface ChangelogRelease {
  version: string;
  date: string | null;
  sections: ChangelogSection[];
}
export interface ReleaseFiles {
  macArm: string;
  macIntel: string;
  windows: string;
  appImage: string;
  deb: string;
  rpm: string;
}
export interface Release {
  version: string;
  date: string | null;
  files: ReleaseFiles;
}
export function readRelease(readme: string, changelog: string): Release;
export function parseChangelog(text: string): ChangelogRelease[];
