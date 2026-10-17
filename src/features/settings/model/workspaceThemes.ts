/** Colour themes for the inset workspace. Each one paints the shell around the
 * card, the card itself, text, accent and the markdown link/heading colours,
 * with its own dark and light palette. "Default" sets nothing, so the
 * stylesheet's neutral greys and the hue/saturation tint stay in charge. */

export type WorkspaceThemeId =
  | "default"
  | "harbor"
  | "moss"
  | "ember"
  | "orchid"
  | "tidepool"
  | "dune"
  | "rosewood"
  | "glacier"
  | "arcade"
  | "ink";

export type WorkspacePalette = {
  /** The shell behind the sidebar, tabs and footer. Also the native glass tint. */
  surface: string;
  /** The card, as painted over the window glass (dark mode only uses glass). */
  panel: string;
  /** The card when the window is opaque. */
  panelSolid: string;
  content: string;
  accent: string;
  link: string;
  heading: string;
};

export type WorkspaceTheme = {
  id: WorkspaceThemeId;
  label: string;
  dark: WorkspacePalette;
  light: WorkspacePalette;
};

export const WORKSPACE_THEME_DEFAULT: WorkspaceThemeId = "default";

// The default palette mirrors the stylesheet's own values; it is only read for
// previews and the native tint, never written as variables.
export const WORKSPACE_THEMES: readonly WorkspaceTheme[] = [
  {
    id: "default",
    label: "Default",
    dark: {
      surface: "#0d0d0d",
      panel: "#060606",
      panelSolid: "#141414",
      content: "#ebebeb",
      accent: "#4da3f5",
      link: "#7dd3fc",
      heading: "#f9a8c9",
    },
    light: {
      surface: "#f5f5f5",
      panel: "#ffffff",
      panelSolid: "#ffffff",
      content: "#2e2e2e",
      accent: "#4da3f5",
      link: "#0866c6",
      heading: "#be185d",
    },
  },
  {
    id: "harbor",
    label: "Harbor",
    dark: {
      surface: "#0b1018",
      panel: "#060a10",
      panelSolid: "#121a26",
      content: "#dbe4f0",
      accent: "#5b9cf2",
      link: "#8cc4ff",
      heading: "#a3b6ff",
    },
    light: {
      surface: "#eef2f7",
      panel: "#ffffff",
      panelSolid: "#ffffff",
      content: "#1b2533",
      accent: "#2f6fd0",
      link: "#1f5fbf",
      heading: "#3b4fb8",
    },
  },
  {
    id: "moss",
    label: "Moss",
    dark: {
      surface: "#0d110e",
      panel: "#070a08",
      panelSolid: "#151b17",
      content: "#dde5dc",
      accent: "#6fbf73",
      link: "#9ad6a0",
      heading: "#c3d98a",
    },
    light: {
      surface: "#f1f4ee",
      panel: "#ffffff",
      panelSolid: "#ffffff",
      content: "#1f2a20",
      accent: "#3d8a45",
      link: "#2e7a3a",
      heading: "#5d7a1c",
    },
  },
  {
    id: "ember",
    label: "Ember",
    dark: {
      surface: "#120e0c",
      panel: "#0a0706",
      panelSolid: "#1c1613",
      content: "#ecdfd6",
      accent: "#f08a4b",
      link: "#f6b07c",
      heading: "#f3a37a",
    },
    light: {
      surface: "#f7f2ee",
      panel: "#ffffff",
      panelSolid: "#ffffff",
      content: "#2d211b",
      accent: "#c75c1e",
      link: "#b4501a",
      heading: "#a8461f",
    },
  },
  {
    id: "orchid",
    label: "Orchid",
    dark: {
      surface: "#110d16",
      panel: "#09070c",
      panelSolid: "#1a1522",
      content: "#e6def0",
      accent: "#b57cf0",
      link: "#cfa6ff",
      heading: "#f2a2d6",
    },
    light: {
      surface: "#f4f0f8",
      panel: "#ffffff",
      panelSolid: "#ffffff",
      content: "#261e33",
      accent: "#8048c8",
      link: "#6f3bb8",
      heading: "#a83a85",
    },
  },
  {
    id: "tidepool",
    label: "Tidepool",
    dark: {
      surface: "#0a1213",
      panel: "#050a0b",
      panelSolid: "#111c1e",
      content: "#d8e8e8",
      accent: "#3fc1b5",
      link: "#7fdcd3",
      heading: "#8ad0e8",
    },
    light: {
      surface: "#edf5f5",
      panel: "#ffffff",
      panelSolid: "#ffffff",
      content: "#162a2b",
      accent: "#138a80",
      link: "#0f7a71",
      heading: "#1d6f8f",
    },
  },
  {
    id: "dune",
    label: "Dune",
    dark: {
      surface: "#13110d",
      panel: "#0b0a07",
      panelSolid: "#1d1a14",
      content: "#ebe3d2",
      accent: "#d9b25c",
      link: "#e8cc8a",
      heading: "#e5a86c",
    },
    light: {
      surface: "#f6f1e7",
      panel: "#fffdf8",
      panelSolid: "#fffdf8",
      content: "#2e2819",
      accent: "#a57d1e",
      link: "#8c6914",
      heading: "#9c5a1c",
    },
  },
  {
    id: "rosewood",
    label: "Rosewood",
    dark: {
      surface: "#140e10",
      panel: "#0b0809",
      panelSolid: "#1e1518",
      content: "#efdfe3",
      accent: "#e7798f",
      link: "#f2a6b4",
      heading: "#f0b38a",
    },
    light: {
      surface: "#f8f0f2",
      panel: "#ffffff",
      panelSolid: "#ffffff",
      content: "#2f1f24",
      accent: "#c0405c",
      link: "#a8344f",
      heading: "#a3502a",
    },
  },
  {
    id: "glacier",
    label: "Glacier",
    dark: {
      surface: "#0e1114",
      panel: "#080a0c",
      panelSolid: "#161a1e",
      content: "#e1e7ec",
      accent: "#7fb4d9",
      link: "#a7d0ec",
      heading: "#b9c8e8",
    },
    light: {
      surface: "#f2f5f8",
      panel: "#ffffff",
      panelSolid: "#ffffff",
      content: "#20262d",
      accent: "#3a7aa8",
      link: "#2d6a96",
      heading: "#44598f",
    },
  },
  {
    id: "arcade",
    label: "Arcade",
    dark: {
      surface: "#0f0b1a",
      panel: "#08060f",
      panelSolid: "#181229",
      content: "#ece6ff",
      accent: "#ff5fd2",
      link: "#5ff2ff",
      heading: "#ffd35f",
    },
    light: {
      surface: "#f5f2fc",
      panel: "#ffffff",
      panelSolid: "#ffffff",
      content: "#231a3a",
      accent: "#c2189a",
      link: "#0a7f90",
      heading: "#9a6608",
    },
  },
  {
    id: "ink",
    label: "Ink",
    dark: {
      surface: "#000000",
      panel: "#000000",
      panelSolid: "#0a0a0a",
      content: "#f5f5f5",
      accent: "#7aa7ff",
      link: "#9cc0ff",
      heading: "#ffffff",
    },
    light: {
      surface: "#ffffff",
      panel: "#ffffff",
      panelSolid: "#ffffff",
      content: "#000000",
      accent: "#0050d8",
      link: "#0040b0",
      heading: "#000000",
    },
  },
];

const BY_ID = new Map(WORKSPACE_THEMES.map((theme) => [theme.id, theme]));

export function isWorkspaceThemeId(value: unknown): value is WorkspaceThemeId {
  return typeof value === "string" && BY_ID.has(value as WorkspaceThemeId);
}

export function workspaceTheme(id: WorkspaceThemeId): WorkspaceTheme {
  return BY_ID.get(id) ?? WORKSPACE_THEMES[0];
}

const PALETTE_KEYS = [
  "surface",
  "panel",
  "panelSolid",
  "content",
  "accent",
  "link",
  "heading",
] as const satisfies readonly (keyof WorkspacePalette)[];

function variableName(scheme: "dark" | "light", key: string) {
  const kebab = key.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
  return `--wt-${scheme}-${kebab}`;
}

/** The `--wt-*` variables the stylesheet reads under `html.layout-inset`, or
 * none for the default theme. */
export function workspaceThemeVariables(
  id: WorkspaceThemeId,
): Record<string, string> {
  if (id === WORKSPACE_THEME_DEFAULT) return {};
  const theme = workspaceTheme(id);
  const variables: Record<string, string> = {};
  for (const scheme of ["dark", "light"] as const) {
    for (const key of PALETTE_KEYS) {
      variables[variableName(scheme, key)] = theme[scheme][key];
    }
  }
  return variables;
}

/** Every variable any theme can set, so switching themes clears the last one. */
export const WORKSPACE_THEME_VARIABLE_NAMES: readonly string[] = (
  ["dark", "light"] as const
).flatMap((scheme) => PALETTE_KEYS.map((key) => variableName(scheme, key)));
