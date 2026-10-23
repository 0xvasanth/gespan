import { describe, expect, it } from "vitest";
import type { Block } from "./session";
import { rebaseToolCall, rebaseWorkSummary } from "./rebaseToolCall";

function shell(text: string): Block {
  return { id: text, role: "tool", text, tool: { kind: "shell" } };
}

describe("Rebase CLI tool calls", () => {
  it("recognizes app actions with absolute, quoted, or bare executables", () => {
    expect(
      rebaseToolCall(
        shell(
          "/repo/target/debug/Rebase.app/Contents/MacOS/rebase app notes.list --json '{}'",
        ),
      )?.label,
    ).toBe("List notes");
    expect(
      rebaseToolCall(
        shell(
          "'/Applications/Rebase App/rebase' app folders.move --input -",
        ),
      )?.label,
    ).toBe("Move a session");
    expect(
      rebaseToolCall(
        shell('"C:\\Program Files\\Rebase\\rebase.exe" app notes.list'),
      )?.label,
    ).toBe("List notes");
    expect(rebaseToolCall(shell("rebase app --help"))?.label).toBe(
      "View CLI commands",
    );
    expect(
      rebaseToolCall(shell("rebase app sessions.read --json '{}'"))?.label,
    ).toBe("Read a session");
    expect(
      rebaseToolCall(shell("rebase app sessions.send --json '{}'"))?.label,
    ).toBe("Continue a session");
    expect(
      rebaseToolCall(shell("rebase app sessions.draft --json '{}'"))?.label,
    ).toBe("Save a draft");
    for (const [action, label] of [
      ["sessions.stop", "Stop a session"],
      ["sessions.archive", "Archive a session"],
      ["sessions.delete", "Delete a session"],
    ]) {
      expect(
        rebaseToolCall(shell(`rebase app ${action} --json '{}'`))?.label,
      ).toBe(label);
    }
    expect(
      rebaseToolCall(shell("rebase app notes.write --input -"))?.label,
    ).toBe("Write a note");
    expect(
      rebaseToolCall({
        id: "generic",
        role: "tool",
        text: "Run command:",
        tool: { kind: "other", title: "rebase app sessions.start" },
      })?.label,
    ).toBe("Start a session");
    expect(
      rebaseToolCall({
        id: "codex-action",
        role: "tool",
        text: "List",
        tool: {
          kind: "execute",
          preview: { kind: "shell", title: "rebase app notes.list" },
        },
      })?.label,
    ).toBe("List notes");
  });

  it("does not restyle unrelated commands or text mentioning the CLI", () => {
    expect(
      rebaseToolCall(shell("echo rebase app notes.list")),
    ).toBeUndefined();
    expect(rebaseToolCall(shell("rebase control list"))).toBeUndefined();
    expect(
      rebaseToolCall({
        id: "prose",
        role: "assistant",
        text: "rebase app notes.list",
      }),
    ).toBeUndefined();
  });

  it("does not compact compound shell commands or hide a longer shell preview", () => {
    for (const command of [
      "rebase app notes.list && echo extra",
      "rebase app notes.list; echo extra",
      "rebase app notes.list | cat",
      "rebase app notes.list\necho extra",
      "rebase app notes.list --json \"$(echo extra)\"",
    ]) {
      expect(rebaseToolCall(shell(command))).toBeUndefined();
    }
    expect(
      rebaseToolCall({
        id: "short-title",
        role: "tool",
        text: "rebase app notes.list",
        tool: {
          kind: "shell",
          title: "rebase app notes.list",
          preview: {
            kind: "shell",
            title: "rebase app notes.list && echo extra",
          },
        },
      }),
    ).toBeUndefined();
    expect(
      rebaseToolCall(
        shell("rebase app sessions.send --json '{\"prompt\":\"a; b\"}'"),
      )?.command,
    ).toBe("rebase app sessions.send --json '{\"prompt\":\"a; b\"}'");
  });

  it("names a group only when all its tool calls use Rebase", () => {
    const calls = [
      shell("rebase app --help"),
      shell("rebase app notes.list"),
    ];
    expect(rebaseWorkSummary(calls, true)).toBe("Using Rebase");
    expect(rebaseWorkSummary(calls, false)).toBe("Used Rebase");
    expect(
      rebaseWorkSummary([...calls, shell("git status")], true),
    ).toBeUndefined();
  });
});
