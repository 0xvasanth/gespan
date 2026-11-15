/** The repository's guides, as listed in README.md's Documentation table. */
export const REPO = "https://github.com/0xvasanth/gespan";
export const doc = (file: string) => `${REPO}/blob/main/docs/${file}`;

export const GUIDES = {
  providers: doc("providers.md"),
  board: doc("board.md"),
  actions: doc("actions.md"),
  agentAccess: doc("agent-access.md"),
  remote: doc("remote-access.md"),
  jira: doc("jira.md"),
  install: doc("install.md"),
  privacy: doc("privacy.md"),
  building: doc("building.md"),
  desktop: doc("desktop/README.md"),
  mobile: `${REPO}/blob/main/mobile/README.md#pair-with-desktop`,
} as const;
