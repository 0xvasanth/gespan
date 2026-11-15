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
  tag: string;
  date: string | null;
  url: string;
  files: ReleaseFiles;
}
export interface GitHubRelease {
  tag_name: string;
  draft?: boolean;
  prerelease?: boolean;
  published_at?: string | null;
  html_url: string;
  assets?: { name: string; browser_download_url: string }[];
}
export function releaseFromGitHub(release: GitHubRelease): Release;
export function compareVersions(a: string, b: string): number;
export function parseChangelog(text: string, published?: string): ChangelogRelease[];
