import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import {
  WorkspaceLine,
  WorkspaceMenu,
} from "@/features/workspace/WorkspaceMenu.android";
import type { ChangedFile } from "@/features/workspace/types";

vi.mock("react-native", () => ({
  View: ({ children, style }: { children?: ReactNode; style?: object }) => (
    <div style={style}>{children}</div>
  ),
  Text: ({
    children,
    style,
    ellipsizeMode,
    numberOfLines,
    accessibilityLabel,
  }: {
    children?: ReactNode;
    style?: object;
    ellipsizeMode?: string;
    numberOfLines?: number;
    accessibilityLabel?: string;
  }) => (
    <span
      aria-label={accessibilityLabel}
      style={style}
      data-ellipsis={ellipsizeMode}
      data-lines={numberOfLines}
    >
      {children}
    </span>
  ),
}));
vi.mock("@/ui/ChoiceMenu", () => ({
  ChoiceMenu: ({
    children,
    style,
  }: {
    children?: ReactNode;
    style?: object;
  }) => <button style={style}>{children}</button>,
}));
vi.mock("@/ui/Icon", () => ({ Icon: () => null }));
vi.mock("@/ui/theme", () => ({
  useColors: () => ({ content: "white", muted: "gray" }),
}));
vi.mock("@/features/changes/diffTheme", () => ({
  formatCount: String,
  useDiffTheme: () => ({ add: { fg: "green" }, del: { fg: "red" } }),
}));

it("bounds long branch labels in empty-session and chat workspace lines while preserving counts", () => {
  (
    globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  const root = createRoot(container);
  const branch = "feat/dispatch_and_benchmarks_and_a_very_long_branch_suffix";
  const files: ChangedFile[] = [
    {
      relative: "app.tsx",
      status: "modified",
      additions: 12,
      deletions: 3,
      lines: [],
    },
  ];
  try {
    for (const empty of [true, false]) {
      act(() =>
        root.render(
          empty ? (
            <WorkspaceMenu
              value={{ mode: "current" }}
              worktrees={[]}
              onChange={() => {}}
              branch={branch}
              files={files}
              loading={false}
              onOpenChanges={() => {}}
            />
          ) : (
            <WorkspaceLine current branch={branch} files={files} />
          ),
        ),
      );
      const label = container.querySelector<HTMLElement>(
        `[aria-label="${branch}"]`,
      )!;
      expect(label.textContent).toBe(branch);
      expect(label.dataset.ellipsis).toBe("middle");
      expect(label.dataset.lines).toBe("1");
      expect(label.style.maxWidth).toBe("160px");
      expect(label.style.flexShrink).toBe("1");
      expect(label.parentElement!.style.maxWidth).toBe("100%");
      expect(container.textContent).toContain("1 files+12-3");
      if (empty)
        expect(container.querySelector("button")!.style.maxWidth).toBe("100%");
    }
  } finally {
    act(() => root.unmount());
  }
});
