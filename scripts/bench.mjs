// Development-mode benchmark for the native app. Builds the dev app the way
// `bun run dev` does, runs it against seeded, isolated data with stub agent
// CLIs, and writes JSON results plus a summary, compared with the stored
// baseline. See docs/diagnostics/benchmarks.md.
//
//   bun run bench                 # full suite: startup, interaction, 15 min soak
//   bun run bench --soak 2m       # shorter soak
//   bun run bench --quick         # 2 startups, 5 cycles, 60 s soak
//   bun run bench --save-baseline # also store this run as the baseline
import { spawn, execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync, chmodSync } from "node:fs";
import { cpus, homedir, loadavg, totalmem } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  collectRun, compare, median, memoryByLabel, memoryGrowth, parseDuration, phaseStats, round,
  sameMachine, soakGrowth, summarize,
} from "./bench-lib.mjs";

const workspace = fileURLToPath(new URL("../", import.meta.url));
const MANIFEST = "crates/gespan-gpui/Cargo.toml";
export const DEFAULT_BASELINE = "crates/gespan-gpui/bench/baseline.json";
const PHASES = ["tab_switch", "chat_scroll", "panes"];
/** No event from the app for this long means it hung. */
const SILENCE_MS = 90_000;
const TIMING = /^(startup\.|tab_switch|chat_scroll|panes|memory\.idle)/;

export function parseBenchArgs(argv) {
  const options = {
    startupRuns: 5, cycles: 20, soakSecs: 15 * 60, only: new Set(["startup", "interaction", "soak"]),
    build: true, saveBaseline: false, baseline: DEFAULT_BASELINE, out: undefined,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const value = () => {
      if (i + 1 >= argv.length) throw new Error(`${arg} needs a value.`);
      return argv[++i];
    };
    if (arg === "--quick") Object.assign(options, { startupRuns: 2, cycles: 5, soakSecs: 60 });
    else if (arg === "--soak") options.soakSecs = parseDuration(value());
    else if (arg === "--startup-runs") options.startupRuns = positive(arg, value());
    else if (arg === "--cycles") options.cycles = positive(arg, value());
    else if (arg === "--only") {
      const parts = value().split(",").map((p) => p.trim());
      for (const part of parts) {
        if (!["startup", "interaction", "soak"].includes(part)) throw new Error(`Unknown stage "${part}".`);
      }
      options.only = new Set(parts);
    } else if (arg === "--no-build") options.build = false;
    else if (arg === "--save-baseline") options.saveBaseline = true;
    else if (arg === "--baseline") options.baseline = value();
    else if (arg === "--out") options.out = value();
    else throw new Error(`Unknown option ${arg}. See docs/diagnostics/benchmarks.md.`);
  }
  if (options.soakSecs === 0) options.only.delete("soak");
  return options;
}

function positive(flag, text) {
  const n = Number(text);
  if (!Number.isInteger(n) || n < 1) throw new Error(`${flag} needs a positive whole number.`);
  return n;
}

const log = (message) => console.log(`[bench] ${message}`);

function run(command, args, options = {}) {
  return new Promise((resolvePromise) => {
    const child = spawn(command, args, { cwd: workspace, stdio: "inherit", ...options });
    child.once("error", (error) => resolvePromise({ code: 1, error }));
    child.once("close", (code, signal) => resolvePromise({ code, signal }));
  });
}

async function buildDevApp() {
  const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--manifest-path", MANIFEST, "--locked", "--no-deps", "--format-version", "1"], { cwd: workspace, encoding: "utf8" }));
  const pkg = metadata.packages.find((p) => p.name === "gespan-gpui");
  const target = metadata.target_directory;
  log("Building the dev app (native-dev profile)…");
  const built = await run("cargo", ["build", "--manifest-path", MANIFEST, "--locked", "-p", "gespan-gpui", "--bin", "gespan-gpui", "--profile", "native-dev"]);
  if (built.code !== 0) throw new Error("cargo build failed.");
  // The same bundle `bun run dev` launches: Gespan Dev, development data.
  const bundled = await run("bash", ["scripts/bundle-macos.sh", "native-dev", target, pkg.version, "dev-data"]);
  if (bundled.code !== 0) throw new Error("Packaging Gespan Dev.app failed.");
  return devExecutable(target);
}

function devExecutable(target) {
  return join(target, "native-dev", "Gespan Dev.app", "Contents", "MacOS", "gespan-gpui");
}

function targetDirectory() {
  const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--manifest-path", MANIFEST, "--locked", "--no-deps", "--format-version", "1"], { cwd: workspace, encoding: "utf8" }));
  return metadata.target_directory;
}

/**
 * Agent CLIs that answer `--version` and nothing else: nothing leaves the
 * Mac. The version is above any release, so no update panel covers the chat.
 */
function writeStubAgent(dir) {
  const path = join(dir, "stub-agent");
  writeFileSync(path, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo '999.0.0'; fi\nexit 0\n");
  chmodSync(path, 0o755);
  return path;
}

function prepareProject(dir) {
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "README.md"), "# Benchmark project\n");
  execFileSync("git", ["init", "-q"], { cwd: dir });
  execFileSync("git", ["-c", "user.name=bench", "-c", "user.email=bench@localhost", "add", "-A"], { cwd: dir });
  execFileSync("git", ["-c", "user.name=bench", "-c", "user.email=bench@localhost", "commit", "-qm", "init"], { cwd: dir });
}

/**
 * Launches the dev app once and collects its stderr until it quits. The
 * app is killed if it goes silent for SILENCE_MS or outlives `limitMs`.
 */
function launch(executable, { mode, data, project, stub, env = {}, limitMs }) {
  return new Promise((resolvePromise) => {
    const lines = [];
    const spawnedAt = Date.now();
    const child = spawn(executable, ["--dev-data", "--cwd", project], {
      cwd: project,
      stdio: ["ignore", "ignore", "pipe"],
      env: {
        ...process.env,
        GESPAN_BENCH: mode,
        GESPAN_NATIVE_DATA_DIR: data,
        GESPAN_CLAUDE_BINARY: stub,
        GESPAN_CODEX_BINARY: stub,
        ZED_MEASUREMENTS: "1",
        RUST_BACKTRACE: "1",
        ...env,
      },
    });
    let lastEvent = Date.now();
    let killedFor;
    const kill = (reason) => {
      if (killedFor) return;
      killedFor = reason;
      child.kill("SIGTERM");
      setTimeout(() => child.kill("SIGKILL"), 5000).unref();
    };
    const watchdog = setInterval(() => {
      if (Date.now() - lastEvent > SILENCE_MS) kill(`no benchmark event for ${SILENCE_MS / 1000}s (hung)`);
      if (limitMs && Date.now() - spawnedAt > limitMs) kill("ran past its time limit");
    }, 1000);
    createInterface({ input: child.stderr }).on("line", (text) => {
      const at = Date.now();
      if (text.startsWith("GESPAN_BENCH ")) lastEvent = at;
      lines.push({ at, text });
    });
    const finish = (code, signal, error) => {
      clearInterval(watchdog);
      const result = collectRun(lines, spawnedAt);
      result.exit = { code, signal, killedFor, error: error?.message };
      result.durationS = round((Date.now() - spawnedAt) / 1000, 1);
      result.tail = lines.slice(-40).map((l) => l.text);
      resolvePromise(result);
    };
    child.once("error", (error) => finish(1, undefined, error));
    child.once("close", (code, signal) => finish(code, signal));
  });
}

/** Why a finished run counts as a crash, or undefined if it ended cleanly. */
export function crashReason(run) {
  if (run.exit.error) return `could not start: ${run.exit.error}`;
  if (run.exit.killedFor) return run.exit.killedFor;
  if (run.exit.signal) return `killed by ${run.exit.signal}`;
  if (run.exit.code !== 0) return `exited with code ${run.exit.code}`;
  if (!run.done) return "quit before the benchmark finished";
  return undefined;
}

function hangReports(data) {
  const dir = join(data, "gpui-hangs");
  return existsSync(dir) ? readdirSync(dir).length : 0;
}

function crashReportsSince(startedAt) {
  const dir = join(homedir(), "Library/Logs/DiagnosticReports");
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => name.startsWith("gespan-gpui"))
    .map((name) => join(dir, name))
    .filter((path) => {
      try { return execFileSync("stat", ["-f", "%m", path], { encoding: "utf8" }) * 1000 >= startedAt; } catch { return false; }
    });
}

function machine() {
  const sysctl = (key) => {
    try { return execFileSync("sysctl", ["-n", key], { encoding: "utf8" }).trim(); } catch { return undefined; }
  };
  return {
    model: sysctl("hw.model"),
    cpu: sysctl("machdep.cpu.brand_string") ?? cpus()[0]?.model,
    cores: cpus().length,
    memory_gb: Math.round(totalmem() / 2 ** 30),
    os: sysctl("kern.osproductversion"),
  };
}

function gitInfo() {
  const git = (...args) => {
    try { return execFileSync("git", args, { cwd: workspace, encoding: "utf8" }).trim(); } catch { return undefined; }
  };
  return { commit: git("rev-parse", "--short", "HEAD"), branch: git("rev-parse", "--abbrev-ref", "HEAD"), dirty: Boolean(git("status", "--porcelain")) };
}

const load = () => loadavg().map((l) => round(l, 1));

function stabilityOf(runs, data) {
  const crashes = runs.filter((r) => crashReason(r));
  return {
    crashes: crashes.length,
    panics: runs.reduce((n, r) => n + r.panics.length, 0),
    hangs: hangReports(data),
    frame_timeouts: runs.reduce((n, r) => n + (r.done?.frame_timeouts ?? 0), 0),
  };
}

async function startupStage(executable, ctx, options) {
  const runs = [];
  // One unmeasured launch warms the disk cache after a fresh build.
  for (let i = 0; i <= options.startupRuns; i++) {
    const data = join(ctx.dir, `startup-${i}`);
    cpSync(ctx.seeded, data, { recursive: true });
    const result = await launch(executable, { mode: "startup", data, project: ctx.project, stub: ctx.stub, limitMs: 120_000 });
    const idle = memoryByLabel(result, "idle");
    const reason = crashReason(result);
    log(`startup ${i === 0 ? "warm-up" : `${i}/${options.startupRuns}`}: interactive ${result.startup.interactive_ms ?? "?"} ms${reason ? ` — CRASH: ${reason}` : ""}`);
    if (i > 0 || reason) runs.push({ ...result.startup, idle_rss_mb: idle?.rss_mb, idle_footprint_mb: idle?.footprint_mb, crash: reason, data, result });
  }
  const measured = runs.filter((r) => !r.crash);
  const pick = (key) => round(median(measured.map((r) => r[key]).filter((v) => v !== undefined)), 1);
  return {
    runs: runs.map(({ result, data, ...rest }) => rest),
    median: Object.fromEntries(["app_run_ms", "first_frame_ms", "interactive_ms", "idle_rss_mb", "idle_footprint_mb"].map((k) => [k, pick(k)])),
    stability: {
      crashes: runs.filter((r) => r.crash).length,
      panics: runs.reduce((n, r) => n + r.result.panics.length, 0),
      hangs: runs.reduce((n, r) => n + hangReports(r.data), 0),
    },
  };
}

async function interactionStage(executable, ctx, options) {
  const data = join(ctx.dir, "interaction");
  cpSync(ctx.seeded, data, { recursive: true });
  log(`interaction: tab switching, long chat scroll, panes, then ${options.cycles} open/close cycles…`);
  const result = await launch(executable, {
    mode: "interaction", data, project: ctx.project, stub: ctx.stub,
    env: { GESPAN_BENCH_CYCLES: String(options.cycles) }, limitMs: 30 * 60_000,
  });
  const reason = crashReason(result);
  if (reason) log(`interaction CRASH: ${reason}`);
  const phases = phaseStats(result, [...PHASES, "cycle"]);
  const inactive = Object.values(phases).reduce((n, p) => n + p.inactive_steps, 0);
  return {
    phases,
    memory: {
      idle: memoryByLabel(result, "idle"),
      before: memoryByLabel(result, "before_cycles"),
      after: memoryByLabel(result, "after_cycles"),
      growth: memoryGrowth(result),
      samples: result.memory,
    },
    layout: result.layout,
    inactive_steps: inactive,
    stability: { ...stabilityOf([result], data), crash: reason, tail: reason ? result.tail : undefined },
  };
}

async function soakStage(executable, ctx, options) {
  const data = join(ctx.dir, "soak");
  cpSync(ctx.seeded, data, { recursive: true });
  const startedAt = Date.now();
  const deadline = startedAt + options.soakSecs * 1000;
  const runs = [];
  const crashDetails = [];
  log(`soak: ${options.soakSecs}s of scripted actions; a crash is recorded and the app relaunched…`);
  while (deadline - Date.now() > 20_000) {
    const secs = Math.round((deadline - Date.now()) / 1000);
    const result = await launch(executable, {
      mode: "soak", data, project: ctx.project, stub: ctx.stub,
      env: { GESPAN_BENCH_SOAK_SECS: String(secs) }, limitMs: (secs + 120) * 1000,
    });
    runs.push(result);
    const reason = crashReason(result);
    if (!reason) break;
    const at = round((Date.now() - startedAt) / 1000, 0);
    log(`soak CRASH at ${at}s: ${reason}`);
    crashDetails.push({ at_s: at, reason, tail: result.tail, reports: crashReportsSince(startedAt) });
  }
  const memory = runs.flatMap((r) => r.memory);
  const { start, end, growth } = soakGrowth(memory);
  return {
    duration_s: round((Date.now() - startedAt) / 1000, 0),
    launches: runs.length,
    iterations: runs.reduce((n, r) => n + (r.events.find((e) => e.event === "soak_end")?.iterations ?? 0), 0),
    phases: phaseStats({ phases: mergePhases(runs) }, ["soak"]),
    memory: {
      start, end, samples: memory, growth,
    },
    stability: stabilityOf(runs, data),
    crash_details: crashDetails,
  };
}

function mergePhases(runs) {
  const merged = {};
  for (const run of runs) {
    for (const [name, bucket] of Object.entries(run.phases)) {
      const into = (merged[name] ??= { frames: [], latency: [], inactive: 0 });
      into.frames.push(...bucket.frames);
      into.latency.push(...bucket.latency);
      into.inactive += bucket.inactive;
    }
  }
  return merged;
}

/** Seeds the chats once; every run starts from a copy, like a returning user. */
async function seedData(executable, ctx) {
  log("Seeding benchmark data (8 chats, one long)…");
  const result = await launch(executable, { mode: "startup", data: ctx.seeded, project: ctx.project, stub: ctx.stub, limitMs: 180_000 });
  const reason = crashReason(result);
  if (reason) throw new Error(`The seeding launch failed: ${reason}\n${result.tail.join("\n")}`);
  // Hang reports and logs from seeding don't belong to the measured runs.
  rmSync(join(ctx.seeded, "gpui-hangs"), { recursive: true, force: true });
}

export async function main(argv = process.argv.slice(2)) {
  if (process.platform !== "darwin") throw new Error("The dev benchmark runs on macOS only (it measures the Gespan Dev app bundle).");
  const options = parseBenchArgs(argv);
  const createdAt = new Date().toISOString();
  const outDir = resolve(workspace, options.out ?? join("output/bench", createdAt.replace(/[:.]/g, "-")));
  const dir = join(outDir, "work");
  mkdirSync(dir, { recursive: true });
  const executable = options.build ? await buildDevApp() : devExecutable(targetDirectory());
  if (!existsSync(executable)) throw new Error(`Missing ${executable}; run without --no-build.`);
  const ctx = { dir, project: join(dir, "project"), seeded: join(dir, "seeded"), stub: writeStubAgent(dir) };
  prepareProject(ctx.project);
  const result = {
    schema: 1,
    created_at: createdAt,
    git: gitInfo(),
    machine: machine(),
    config: { startup_runs: options.startupRuns, cycles: options.cycles, soak_s: options.only.has("soak") ? options.soakSecs : 0, stages: [...options.only] },
    load: { start: load() },
    log_dir: join(homedir(), "Library/Logs/dev.gespan.native.dev"),
    warnings: [],
  };
  await seedData(executable, ctx);
  if (options.only.has("startup")) result.startup = await startupStage(executable, ctx, options);
  if (options.only.has("interaction")) result.interaction = await interactionStage(executable, ctx, options);
  if (options.only.has("soak")) result.soak = await soakStage(executable, ctx, options);
  result.load.end = load();
  if (Math.max(...result.load.start, ...result.load.end) > cpus().length) {
    result.warnings.push(`The load average (${result.load.start[0]} → ${result.load.end[0]}) was above the ${cpus().length} cores: timings are inflated. Rerun on a quieter machine before trusting a regression.`);
  }
  const inactive = (result.interaction?.inactive_steps ?? 0) + (result.soak?.phases?.soak?.inactive_steps ?? 0);
  if (inactive > 0) {
    result.warnings.push(`${inactive} steps ran while the Gespan Dev window was not active; GPUI throttles inactive windows to 30 fps, so leave the window in front during a run.`);
  }

  const baselinePath = resolve(workspace, options.baseline);
  const baseline = existsSync(baselinePath) ? JSON.parse(readFileSync(baselinePath, "utf8")) : undefined;
  const same = sameMachine(result.machine, baseline?.machine);
  const checks = compare(result, baseline).map((c) =>
    c.status === "regressed" && baseline && !same && TIMING.test(c.metric) ? { ...c, status: "warning" } : c);
  result.comparison = { baseline: options.baseline, baseline_created_at: baseline?.created_at, same_machine: same, checks };
  const regressions = checks.filter((c) => c.status === "regressed");

  writeFileSync(join(outDir, "results.json"), JSON.stringify(result, null, 2) + "\n");
  const summary = summarize(result);
  writeFileSync(join(outDir, "summary.md"), summary);
  rmSync(dir, { recursive: true, force: true });
  if (options.saveBaseline) {
    if (regressions.some((c) => c.metric.startsWith("stability."))) {
      log("Not saving a baseline from a run that crashed, panicked or hung.");
    } else {
      mkdirSync(resolve(baselinePath, ".."), { recursive: true });
      const { comparison, ...stored } = result;
      writeFileSync(baselinePath, JSON.stringify(stored, null, 2) + "\n");
      log(`Saved the baseline to ${options.baseline}.`);
    }
  }
  console.log("\n" + summary);
  log(`Results: ${join(outDir, "results.json")}`);
  return regressions.length ? 1 : 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().then((code) => { process.exitCode = code; }, (error) => {
    console.error(`[bench] ${error.message}`);
    process.exitCode = 2;
  });
}
