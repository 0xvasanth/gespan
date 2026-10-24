// @vitest-environment node
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

// The Tauri app is the source of truth. The native GPUI app shares its
// database, webview storage, localStorage keys and mobile wire format, so each
// value is read from the Tauri side and must also appear in the GPUI source.
const root = resolve(__dirname, "../..");
const read = (path: string) => readFileSync(join(root, path), "utf8");

function rustFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return rustFiles(full);
    return full.endsWith(".rs") ? [full] : [];
  });
}

const gpui = ["crates/rebase-core/src", "crates/rebase-gpui/src"]
  .flatMap((dir) => rustFiles(join(root, dir)))
  .map((file) => readFileSync(file, "utf8"))
  .join("\n");

describe("GPUI and Tauri share one set of contracts", () => {
  it("opens the same database under the same app identifier", () => {
    const conf = JSON.parse(read("src-tauri/tauri.conf.json"));
    const identifier: string = conf.identifier;
    const dbName = /data_dir\(\)\?\.join\("([^"]+\.db)"\)/.exec(
      read("src-tauri/src/session_store.rs"),
    )?.[1];
    expect(identifier).toBe("com.rebase.desktop");
    expect(dbName).toBe("rebase.db");
    expect(gpui.includes(`${identifier}/${dbName}`), `${identifier}/${dbName}`).toBe(true);
    expect(gpui.includes(`Library/WebKit/${identifier}`), `Library/WebKit/${identifier}`).toBe(true);
    expect(gpui.includes("com.monocode.desktop"), "com.monocode.desktop").toBe(false);
  });

  it("reads the Pilot tables Tauri creates", () => {
    const tables = [
      ...read("src-tauri/src/pilot_transcript.rs").matchAll(
        /CREATE TABLE IF NOT EXISTS (\w+) \(/g,
      ),
    ].map((m) => m[1]);
    expect(tables.sort()).toEqual(["pilot_blocks", "pilot_transcripts", "pilot_turns"]);
    for (const table of tables) expect(gpui.includes(table), table).toBe(true);
    expect(/\bmono_(transcripts|turns|blocks)\b/.exec(gpui)?.[0]).toBeUndefined();
  });

  it("uses Tauri's localStorage keys", () => {
    const keys = [
      ["src/features/pilots/model/pilot.ts", "rebase:pilot-roster"],
      ["src/features/pilots/model/pilot.ts", "rebase:open-pilot"],
      ["src/features/settings/model/displayPrefs.ts", "rebase.railPilotsPinned"],
      ["src/features/settings/model/settings.ts", "rebase.pilotsEnabled"],
      ["src/features/settings/model/settings.ts", "rebase.pilotMenuBarIcon"],
      ["src/features/settings/model/appearance.ts", "rebase.sidebarTabOrder"],
    ] as const;
    for (const [file, key] of keys) {
      expect(read(file), `${key} in Tauri`).toContain(`"${key}"`);
    }
    // Keys the native app reads must be Tauri's, never the old MonoCode ones.
    for (const key of [
      "rebase:pilot-roster",
      "rebase.railPilotsPinned",
      "rebase.sidebarTabOrder",
    ]) {
      expect(gpui.includes(`"${key}"`), `${key} in GPUI`).toBe(true);
    }
    // Flags are read as `rebase.{name}`; the names must be Tauri's Pilot names.
    for (const name of ["pilotsEnabled", "pilotMenuBarIcon"]) {
      expect(gpui.includes(`"${name}"`), name).toBe(true);
    }
    expect(/railMonosPinned|monosEnabled|monoMenuBarIcon/.exec(gpui)?.[0]).toBeUndefined();
    expect(/"monocode[:.](?!png|local)/.exec(gpui)?.[0]).toBeUndefined();
  });

  it("keeps Pilot files and mobile JSON on the Pilot names", () => {
    expect(read("src-tauri/src/pilot.rs")).toContain('const PILOTS_DIR: &str = "pilots"');
    expect(gpui.includes('.join("pilots")'), '.join("pilots")').toBe(true);
    const access = read("src-tauri/src/mobile_access.rs");
    for (const token of ['catalog["pilots"]', 'strip_prefix("pilot:")', "pilotId", "rebase://pair"]) {
      expect(access, token).toContain(token);
    }
    for (const token of ['"pilots"', 'strip_prefix("pilot:")', "pilotId", "rebase://pair"]) {
      expect(gpui.includes(token), token).toBe(true);
    }
    expect(/monoId|json!.*"monos"|catalog\["monos"\]|strip_prefix\("mono:"\)|monocode:\/\//.exec(gpui)?.[0]).toBeUndefined();
    expect(read("src-tauri/src/pilot_transcript.rs")).toContain("pilotHabit");
    expect(gpui.includes("pilotHabit"), "pilotHabit").toBe(true);
  });

  it("uses Tauri's checkpoint refs", () => {
    expect(read("src-tauri/src/checkpoint/turn.rs")).toContain("refs/rebase/checkpoints/");
    expect(gpui.includes("refs/rebase/checkpoints/"), "refs/rebase/checkpoints/").toBe(true);
    expect(gpui.includes("refs/monocode"), "refs/monocode").toBe(false);
  });

  // Both apps persist the same `agentRun` object and the Claude wake marker row in
  // `sessions.blocks_json`. Renaming any of these on one side silently drops the
  // field when the other app loads or saves the block, so each literal is pinned on
  // both sides. Mirror of `shared_contracts.rs::agent_run_and_wake_marker_json_names_match`.
  it("shares the agentRun fields and the task-notification row with Rebase Native", () => {
    const session = read("src/features/sessions/model/session.ts");
    const store = read("src/features/sessions/data/sessionStore.ts");
    const claude = read("src/integrations/harness/providers/claude/claude.ts");
    const agentRun = /export type AgentRunMeta = \{([\s\S]*?)\n\};/.exec(session)?.[1] ?? "";
    expect(agentRun, "AgentRunMeta in session.ts").not.toBe("");
    const tasks = readFileSync(join(root, "crates/rebase-core/src/tool/tasks.rs"), "utf8");
    for (const field of ["agentId", "resumeOf", "activity", "continuation"]) {
      expect(agentRun, `${field} on AgentRunMeta`).toMatch(new RegExp(`\\b${field}\\?:`));
      expect(store, `${field} kept by the Tauri store`).toContain(`"${field}"`);
      // Native reads it from a desktop block, and writes it back under the same name.
      expect(tasks, `${field} read by Rebase Native`).toMatch(
        new RegExp(`agent(?:\\.get\\(|\\[)"${field}"`),
      );
      expect(tasks, `${field} written by Rebase Native`).toMatch(
        new RegExp(`\\("${field}",|value\\["${field}"\\] =`),
      );
    }

    // The row: `role: "system"`, `interjection: { customType }`, `sentAt`.
    expect(/export type InterjectionMeta = \{\s*customType: string;/.test(session)).toBe(true);
    expect(session).toMatch(/\bsentAt\?: number;/);
    const marker = /function emitWakeMarker[\s\S]*?\n\}/.exec(claude)?.[0] ?? "";
    expect(marker).toContain('customType: "task-notification"');
    expect(marker).toContain("sentAt:");
    const blocks = readFileSync(join(root, "crates/rebase-core/src/store/shared/blocks.rs"), "utf8");
    expect(blocks).toContain('block["interjection"]["customType"]');
    expect(blocks).toContain('json!({"customType":note.custom_type})');
    expect(blocks).toContain('block["sentAt"]');
    expect(gpui).toContain('custom_type: "task-notification".into()');
  });
});
