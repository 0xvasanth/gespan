import { describe, expect, it } from "vitest";
import type { Block } from "./session";
import { gespanToolCall, gespanWorkSummary } from "./gespanToolCall";

function shell(text: string): Block {
  return { id: text, role: "tool", text, tool: { kind: "shell" } };
}

describe("Gespan CLI tool calls", () => {
  it("recognizes app actions with absolute, quoted, or bare executables", () => {
    expect(
      gespanToolCall(
        shell(
          "/repo/target/debug/Gespan.app/Contents/MacOS/gespan app notes.list --json '{}'",
        ),
      )?.label,
    ).toBe("List notes");
    expect(
      gespanToolCall(
        shell(
          "'/Applications/Gespan App/gespan' app folders.move --input -",
        ),
      )?.label,
    ).toBe("Move a session");
    expect(
      gespanToolCall(
        shell('"C:\\Program Files\\Gespan\\gespan.exe" app notes.list'),
      )?.label,
    ).toBe("List notes");
    expect(gespanToolCall(shell("gespan app --help"))?.label).toBe(
      "View CLI commands",
    );
    expect(
      gespanToolCall(shell("gespan app sessions.read --json '{}'"))?.label,
    ).toBe("Read a session");
    expect(
      gespanToolCall(shell("gespan app sessions.send --json '{}'"))?.label,
    ).toBe("Continue a session");
    expect(
      gespanToolCall(shell("gespan app sessions.draft --json '{}'"))?.label,
    ).toBe("Save a draft");
    for (const [action, label] of [
      ["sessions.stop", "Stop a session"],
      ["sessions.archive", "Archive a session"],
      ["sessions.delete", "Delete a session"],
    ]) {
      expect(
        gespanToolCall(shell(`gespan app ${action} --json '{}'`))?.label,
      ).toBe(label);
    }
    expect(
      gespanToolCall(shell("gespan app notes.write --input -"))?.label,
    ).toBe("Write a note");
    expect(
      gespanToolCall({
        id: "generic",
        role: "tool",
        text: "Run command:",
        tool: { kind: "other", title: "gespan app sessions.start" },
      })?.label,
    ).toBe("Start a session");
    expect(
      gespanToolCall({
        id: "codex-action",
        role: "tool",
        text: "List",
        tool: {
          kind: "execute",
          preview: { kind: "shell", title: "gespan app notes.list" },
        },
      })?.label,
    ).toBe("List notes");
  });

  it("does not restyle unrelated commands or text mentioning the CLI", () => {
    expect(
      gespanToolCall(shell("echo gespan app notes.list")),
    ).toBeUndefined();
    expect(gespanToolCall(shell("gespan control list"))).toBeUndefined();
    expect(
      gespanToolCall({
        id: "prose",
        role: "assistant",
        text: "gespan app notes.list",
      }),
    ).toBeUndefined();
  });

  it("does not compact compound shell commands or hide a longer shell preview", () => {
    for (const command of [
      "gespan app notes.list && echo extra",
      "gespan app notes.list; echo extra",
      "gespan app notes.list | cat",
      "gespan app notes.list\necho extra",
      "gespan app notes.list --json \"$(echo extra)\"",
    ]) {
      expect(gespanToolCall(shell(command))).toBeUndefined();
    }
    expect(
      gespanToolCall({
        id: "short-title",
        role: "tool",
        text: "gespan app notes.list",
        tool: {
          kind: "shell",
          title: "gespan app notes.list",
          preview: {
            kind: "shell",
            title: "gespan app notes.list && echo extra",
          },
        },
      }),
    ).toBeUndefined();
    expect(
      gespanToolCall(
        shell("gespan app sessions.send --json '{\"prompt\":\"a; b\"}'"),
      )?.command,
    ).toBe("gespan app sessions.send --json '{\"prompt\":\"a; b\"}'");
  });

  it("names a group only when all its tool calls use Gespan", () => {
    const calls = [
      shell("gespan app --help"),
      shell("gespan app notes.list"),
    ];
    expect(gespanWorkSummary(calls, true)).toBe("Using Gespan");
    expect(gespanWorkSummary(calls, false)).toBe("Used Gespan");
    expect(
      gespanWorkSummary([...calls, shell("git status")], true),
    ).toBeUndefined();
  });
});
