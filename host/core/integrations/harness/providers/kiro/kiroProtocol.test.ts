import { describe, expect, it } from "vitest";
import type { Attachment } from "../../../../features/sessions/model/session";
import {
  KIRO_AUTH_HELP,
  KIRO_LAUNCH_ARGS,
  kiroAutoPermission,
  kiroInitCapabilities,
  kiroIsReplay,
  kiroModelControl,
  kiroPermissionChoice,
  kiroPermissionOptions,
  kiroPromptBlocks,
  kiroSessionId,
  kiroStartupError,
  kiroStderrAuthError,
  kiroStopReason,
  kiroTurnErrorMessage,
  modelsFromKiroSession,
} from "./kiroProtocol";

const modelOption = {
  id: "model",
  name: "Model",
  category: "model",
  type: "select",
  currentValue: "claude-sonnet-4.5",
  options: [
    { value: "auto", name: "Auto" },
    { value: "claude-sonnet-4.5", name: "Claude Sonnet 4.5" },
    { value: "claude-opus-4.5", name: "Claude Opus 4.5" },
  ],
};

const image: Attachment = {
  id: "a1",
  name: "shot.png",
  mimeType: "image/png",
  kind: "image",
  size: 4,
  path: "/tmp/shot.png",
  data: "AAAA",
};

describe("launch args", () => {
  it("tries the V3 engine first and falls back to a bare acp", () => {
    expect(KIRO_LAUNCH_ARGS).toEqual([["acp", "--agent-engine=v3"], ["acp"]]);
  });
});

describe("kiroSessionId", () => {
  it("accepts sessionId and session_id and trims", () => {
    expect(kiroSessionId({ sessionId: " s-1 " })).toBe("s-1");
    expect(kiroSessionId({ session_id: "s-2" })).toBe("s-2");
  });

  it("rejects blanks and non-objects", () => {
    expect(kiroSessionId({ sessionId: "  " })).toBeUndefined();
    expect(kiroSessionId(undefined)).toBeUndefined();
    expect(kiroSessionId("s-3")).toBeUndefined();
  });
});

describe("kiroInitCapabilities", () => {
  it("reads loadSession and prompt capabilities", () => {
    expect(
      kiroInitCapabilities({
        agentCapabilities: {
          loadSession: true,
          promptCapabilities: { image: true, embeddedContext: true },
        },
      }),
    ).toEqual({ loadSession: true, image: true, embeddedContext: true });
  });

  it("defaults everything to false when nothing is advertised", () => {
    const none = { loadSession: false, image: false, embeddedContext: false };
    expect(kiroInitCapabilities({})).toEqual(none);
    expect(kiroInitCapabilities(undefined)).toEqual(none);
    expect(kiroInitCapabilities({ agentCapabilities: { loadSession: "yes" } })).toEqual(
      none,
    );
  });
});

describe("kiroPromptBlocks", () => {
  it("sends images as image blocks when Kiro advertises image prompts", () => {
    const blocks = kiroPromptBlocks("look", [image], { image: true });
    expect(blocks.map((block) => block.type)).toEqual(["text", "image"]);
  });

  it("downgrades an image to a path note when image prompts are not advertised", () => {
    const blocks = kiroPromptBlocks("look", [image], { image: false });
    expect(blocks.map((block) => block.type)).toEqual(["text", "text"]);
    const note = blocks[1] as { type: "text"; text: string };
    expect(note.text).toContain("/tmp/shot.png");
  });

  it("fails loudly instead of dropping an image that has no file to point at", () => {
    const pasted = { ...image, path: undefined };
    expect(() => kiroPromptBlocks("look", [pasted], { image: false })).toThrow(
      /shot\.png/,
    );
  });

  it("points each downgraded image at its own file, in attachment order", () => {
    const second = { ...image, id: "a2", name: "two.png", path: "/tmp/two.png" };
    const blocks = kiroPromptBlocks("look", [image, second], { image: false });
    const notes = blocks.slice(1).map((block) => (block as { text: string }).text);
    expect(notes[0]).toContain("/tmp/shot.png");
    expect(notes[1]).toContain("/tmp/two.png");
  });

  it("does not pair an image with an earlier non-image attachment that carries data", () => {
    const blob: Attachment = {
      id: "b",
      name: "log.txt",
      mimeType: "text/plain",
      kind: "file",
      size: 3,
      path: "/tmp/log.txt",
      data: "QUJD",
    };
    const blocks = kiroPromptBlocks("look", [blob, image], { image: false });
    const text = blocks
      .map((block) => (block.type === "text" ? block.text : ""))
      .join("\n");
    expect(text).toContain("Attached image (read from disk): \"/tmp/shot.png\"");
    expect(text).not.toContain("Attached image (read from disk): \"/tmp/log.txt\"");
  });

  it("downgrades an image sent without any prompt text", () => {
    const blocks = kiroPromptBlocks("", [image], { image: false });
    expect(blocks).toHaveLength(2);
    expect((blocks[1] as { text: string }).text).toContain("/tmp/shot.png");
  });

  it("keeps non-image files as resource links regardless of capabilities", () => {
    const doc: Attachment = {
      id: "d",
      name: "notes.md",
      mimeType: "text/markdown",
      kind: "file",
      size: 3,
      path: "/tmp/notes.md",
    };
    const blocks = kiroPromptBlocks("read", [doc], { image: false });
    expect(blocks.map((block) => block.type)).toEqual(["text", "resource_link"]);
  });
});

describe("kiroModelControl", () => {
  it("prefers a model config option (V3)", () => {
    expect(kiroModelControl({ sessionId: "s", configOptions: [modelOption] })).toEqual({
      kind: "config",
      configId: "model",
      current: "claude-sonnet-4.5",
    });
  });

  it("finds the model option by category and the configId spelling", () => {
    const control = kiroModelControl({
      configOptions: [
        { configId: "mode", category: "mode", currentValue: "default" },
        { configId: "llm", category: "model", currentValue: "x", options: [] },
      ],
    });
    expect(control).toEqual({ kind: "config", configId: "llm", current: "x" });
  });

  it("falls back to the models state (v2)", () => {
    expect(
      kiroModelControl({
        models: {
          currentModelId: "auto",
          availableModels: [{ modelId: "auto", name: "Auto" }],
        },
      }),
    ).toEqual({ kind: "setModel", current: "auto" });
  });

  it("is none when the session advertises no model control", () => {
    expect(kiroModelControl({ sessionId: "s" })).toEqual({ kind: "none" });
    expect(kiroModelControl(undefined)).toEqual({ kind: "none" });
    expect(
      kiroModelControl({ configOptions: [{ id: "mode", category: "mode" }] }),
    ).toEqual({ kind: "none" });
  });
});

describe("modelsFromKiroSession", () => {
  it("lists config-option models with the current one first", () => {
    const models = modelsFromKiroSession({ configOptions: [modelOption] });
    expect(models.map((model) => model.nativeId)).toEqual([
      "claude-sonnet-4.5",
      "auto",
      "claude-opus-4.5",
    ]);
    expect(models[0]).toMatchObject({
      id: "kiro:claude-sonnet-4.5",
      harness: "kiro",
      name: "Claude Sonnet 4.5",
    });
  });

  it("flattens grouped options and dedupes repeats", () => {
    const models = modelsFromKiroSession({
      configOptions: [
        {
          ...modelOption,
          options: [
            { group: "anthropic", name: "Anthropic", options: [{ value: "a", name: "A" }] },
            { value: "a", name: "A again" },
            { value: "b", name: "B" },
          ],
          currentValue: "b",
        },
      ],
    });
    expect(models.map((model) => model.nativeId)).toEqual(["b", "a"]);
  });

  it("reads the models state when there is no config option", () => {
    const models = modelsFromKiroSession({
      models: {
        currentModelId: "m2",
        availableModels: [
          { modelId: "m1", name: "One" },
          { modelId: "m2", name: "Two" },
        ],
      },
    });
    expect(models.map((model) => model.nativeId)).toEqual(["m2", "m1"]);
  });

  it("yields no models for an empty session so the picker keeps its default", () => {
    expect(modelsFromKiroSession({ sessionId: "s" })).toEqual([]);
    expect(modelsFromKiroSession(null)).toEqual([]);
  });
});

describe("kiroStderrAuthError", () => {
  it("promotes sign-in failures with the login hint", () => {
    const message = kiroStderrAuthError("Error: Not logged in");
    expect(message).toContain("Not logged in");
    expect(message).toContain("kiro-cli login");
    expect(kiroStderrAuthError("authentication expired, please log in again")).toContain(
      KIRO_AUTH_HELP,
    );
  });

  it("ignores ordinary log chatter", () => {
    expect(kiroStderrAuthError("")).toBeNull();
    expect(kiroStderrAuthError("[info] checking login state")).toBeNull();
    expect(kiroStderrAuthError("WARN mcp server slow to start")).toBeNull();
    expect(kiroStderrAuthError("Loaded 3 steering files")).toBeNull();
  });
});

describe("kiroStartupError", () => {
  it("appends the login hint to auth-looking failures", () => {
    const error = kiroStartupError(new Error("Not logged in"));
    expect(error.message.endsWith(KIRO_AUTH_HELP)).toBe(true);
  });

  it("explains a timeout as Kiro not starting", () => {
    expect(kiroStartupError(new Error("initialize timed out")).message).toMatch(
      /Kiro CLI did not start/,
    );
  });

  it("keeps the underlying detail for anything else", () => {
    expect(kiroStartupError("spawn ENOENT").message).toBe(
      "Kiro CLI did not start. spawn ENOENT",
    );
  });
});

describe("kiro update metadata", () => {
  it("detects replayed history", () => {
    expect(
      kiroIsReplay({
        update: { sessionUpdate: "agent_message_chunk", _meta: { kiro: { replay: true } } },
      }),
    ).toBe(true);
    expect(kiroIsReplay({ update: { sessionUpdate: "agent_message_chunk" } })).toBe(false);
    expect(kiroIsReplay(null)).toBe(false);
  });
});

const offered = [
  { optionId: "ok-1", name: "Allow once", kind: "allow_once" },
  { optionId: "ok-always", name: "Always allow", kind: "allow_always" },
  { optionId: "no-1", name: "Reject once", kind: "reject_once" },
  { optionId: "no-always", name: "Reject always", kind: "reject_always" },
];

describe("kiro permissions", () => {
  it("reads the offered options with their kinds", () => {
    expect(
      kiroPermissionOptions({
        options: [
          { optionId: "a", name: "A", kind: "allow_once" },
          { option_id: "b", kind: "reject_once" },
          { name: "no id", kind: "allow_once" },
          "junk",
        ],
      }),
    ).toEqual([
      { optionId: "a", name: "A", kind: "allow_once" },
      { optionId: "b", name: "b", kind: "reject_once" },
    ]);
    expect(kiroPermissionOptions({})).toEqual([]);
  });

  it("answers allow with the offered once option, then the offered always option", () => {
    expect(kiroPermissionChoice("allow", offered)).toBe("ok-1");
    expect(kiroPermissionChoice("allow", [offered[1], offered[2]])).toBe("ok-always");
  });

  it("answers deny with the offered reject option", () => {
    expect(kiroPermissionChoice("deny", offered)).toBe("no-1");
    expect(kiroPermissionChoice("deny", [offered[3]])).toBe("no-always");
  });

  it("never invents an option the agent did not offer", () => {
    expect(kiroPermissionChoice("allow", [offered[2], offered[3]])).toBeUndefined();
    expect(kiroPermissionChoice("deny", [offered[0]])).toBeUndefined();
    expect(kiroPermissionChoice("allow", [])).toBeUndefined();
  });

  it("auto-approves by access level using only offered options", () => {
    expect(kiroAutoPermission("full-access", "execute", offered)).toBe("ok-always");
    expect(kiroAutoPermission("auto-accept-edits", "edit", offered)).toBe("ok-1");
    expect(kiroAutoPermission("auto-accept-edits", "read", offered)).toBe("ok-1");
  });

  it("asks the user instead of auto-approving when the access level requires it", () => {
    expect(kiroAutoPermission("supervised", "read", offered)).toBeUndefined();
    expect(kiroAutoPermission("auto-accept-edits", "execute", offered)).toBeUndefined();
    expect(kiroAutoPermission("full-access", "execute", [offered[2]])).toBeUndefined();
  });
});

describe("kiroStopReason", () => {
  it("returns the stop reason of a prompt result", () => {
    expect(kiroStopReason({ stopReason: "refusal" })).toBe("refusal");
    expect(kiroStopReason({ stop_reason: "end_turn" })).toBe("end_turn");
    expect(kiroStopReason({})).toBeUndefined();
    expect(kiroStopReason(undefined)).toBeUndefined();
  });
});

describe("kiroTurnErrorMessage", () => {
  it("adds the login hint to an auth failure and nothing else", () => {
    expect(kiroTurnErrorMessage(new Error("token expired"))).toBe(
      `token expired\n\n${KIRO_AUTH_HELP}`,
    );
    expect(kiroTurnErrorMessage(new Error("rate limited"))).toBe("rate limited");
    expect(kiroTurnErrorMessage("boom")).toBe("boom");
  });
});
