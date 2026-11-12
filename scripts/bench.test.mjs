import assert from "node:assert/strict";
import { test } from "node:test";
import {
  collectRun, compare, memoryGrowth, parseDuration, parseDurationMs, parseLine, percentile,
  phaseStats, sameMachine, soakGrowth, summarize,
} from "./bench-lib.mjs";
import { crashReason, parseBenchArgs } from "./bench.mjs";

const event = (fields) => `GESPAN_BENCH ${JSON.stringify(fields)}`;

test("GPUI frame durations parse in every unit Debug prints", () => {
  assert.equal(parseDurationMs("1.5ms"), 1.5);
  assert.equal(parseDurationMs("834.25µs"), 0.83425);
  assert.equal(parseDurationMs("2.1s"), 2100);
  assert.equal(parseDurationMs("500ns"), 0.0005);
  assert.equal(parseDurationMs("fast"), undefined);
});

test("stderr lines are told apart: events, frames, panics, noise", () => {
  assert.deepEqual(parseLine(event({ event: "done" })), { kind: "event", event: { event: "done" } });
  assert.deepEqual(parseLine("frame duration: 3.2ms"), { kind: "frame", ms: 3.2 });
  assert.equal(parseLine("thread 'main' panicked at src/shell.rs:10:5:").kind, "panic");
  // A truncated event line is ignored rather than ending the run.
  assert.equal(parseLine('GESPAN_BENCH {"event":').kind, "other");
  assert.equal(parseLine("2026-11-12 INFO shell: restored").kind, "other");
});

test("nearest-rank percentiles", () => {
  assert.equal(percentile([], 95), undefined);
  assert.equal(percentile([7], 95), 7);
  const twenty = Array.from({ length: 20 }, (_, i) => i + 1);
  assert.equal(percentile(twenty, 95), 19);
  assert.equal(percentile(twenty, 50), 10);
  assert.equal(percentile([5, 1, 3], 100), 5);
});

test("a run's frames count only inside the phase that produced them", () => {
  const spawnedAt = 1_000;
  const lines = [
    { at: 1_050, text: event({ event: "app_run", wall_ms: 1_040 }) },
    { at: 1_300, text: "frame duration: 40ms" }, // the first, startup frame
    { at: 1_500, text: event({ event: "interactive", wall_ms: 1_480 }) },
    { at: 1_600, text: event({ event: "phase_start", phase: "panes" }) },
    { at: 1_610, text: "frame duration: 4ms" },
    { at: 1_620, text: event({ event: "step", phase: "panes", latency_ms: 20, active: true }) },
    { at: 1_630, text: "frame duration: 300ms" },
    { at: 1_640, text: event({ event: "step", phase: "panes", latency_ms: 310, active: false }) },
    { at: 1_650, text: event({ event: "phase_end", phase: "panes" }) },
    { at: 1_660, text: "frame duration: 99ms" }, // between phases: not counted
    { at: 1_700, text: event({ event: "memory", label: "before_cycles", rss_mb: 200, footprint_mb: 150 }) },
    { at: 1_800, text: event({ event: "memory", label: "after_cycles", rss_mb: 260.5, footprint_mb: 170 }) },
    { at: 1_900, text: event({ event: "done", frame_timeouts: 0 }) },
  ];
  const run = collectRun(lines, spawnedAt);
  assert.deepEqual(run.startup, { app_run_ms: 40, first_frame_ms: 300, interactive_ms: 480 });
  const { panes } = phaseStats(run, ["panes", "chat_scroll"]);
  assert.deepEqual(panes.frame_ms, { count: 2, p50: 4, p95: 300, max: 300 });
  assert.equal(panes.latency_ms.max, 310);
  assert.equal(panes.long_frames, 1);
  assert.equal(panes.inactive_steps, 1);
  assert.deepEqual(memoryGrowth(run), { rss_mb: 60.5, footprint_mb: 20 });
  assert.ok(run.done);
});

const phase = (p95, max = p95, latency = 20) => ({
  frame_ms: { count: 10, p50: 1, p95, max }, latency_ms: { count: 10, p50: 10, p95: latency, max: latency },
});
const result = (overrides = {}) => ({
  startup: { median: { first_frame_ms: 800, interactive_ms: 1000, idle_rss_mb: 300 }, stability: { crashes: 0, panics: 0, hangs: 0 } },
  interaction: {
    phases: { panes: phase(10) },
    memory: { growth: { rss_mb: 10, footprint_mb: 5 } },
    stability: { crashes: 0, panics: 0, hangs: 0 },
  },
  ...overrides,
});
const statusOf = (checks, metric) => checks.find((c) => c.metric === metric)?.status;

test("a regression needs both the relative limit and the absolute floor", () => {
  const baseline = result();
  // +30% on a 1000 ms startup (> +25%, > +100 ms): regressed.
  let now = result({ startup: { ...baseline.startup, median: { ...baseline.startup.median, interactive_ms: 1300 } } });
  assert.equal(statusOf(compare(now, baseline), "startup.interactive_ms"), "regressed");
  // +24%: within the limit.
  now = result({ startup: { ...baseline.startup, median: { ...baseline.startup.median, interactive_ms: 1240 } } });
  assert.equal(statusOf(compare(now, baseline), "startup.interactive_ms"), "ok");
  // Frame p95 1 ms → 2 ms is +100% but under the 2 ms floor: noise, not a regression.
  const tiny = result({ interaction: { ...baseline.interaction, phases: { panes: phase(1) } } });
  const doubled = result({ interaction: { ...baseline.interaction, phases: { panes: phase(2) } } });
  assert.equal(statusOf(compare(doubled, tiny), "panes.frame_p95_ms"), "ok");
  // 10 ms → 14 ms is +40% and +4 ms: regressed.
  const slower = result({ interaction: { ...baseline.interaction, phases: { panes: phase(14) } } });
  assert.equal(statusOf(compare(slower, baseline), "panes.frame_p95_ms"), "regressed");
  // Idle RSS +25% and +75 MB: regressed.
  now = result({ startup: { ...baseline.startup, median: { ...baseline.startup.median, idle_rss_mb: 375 } } });
  assert.equal(statusOf(compare(now, baseline), "memory.idle_rss_mb"), "regressed");
});

test("a frame over 250 ms fails only when it is new", () => {
  const smooth = result();
  const janky = result({ interaction: { ...smooth.interaction, phases: { panes: phase(10, 400) } } });
  assert.equal(statusOf(compare(janky, smooth), "panes.max_frame_ms"), "regressed");
  // The baseline already had one: a known problem, not a regression.
  assert.equal(statusOf(compare(janky, janky), "panes.max_frame_ms"), "ok");
  // With no baseline at all it is new.
  assert.equal(statusOf(compare(janky, undefined), "panes.max_frame_ms"), "regressed");
});

test("crashes, panics, hangs and leaks fail even without a baseline", () => {
  const clean = compare(result(), undefined);
  assert.ok(clean.every((c) => c.status !== "regressed"));
  const crashed = result({ soak: { stability: { crashes: 1, panics: 0, hangs: 0 }, memory: {} } });
  assert.equal(statusOf(compare(crashed, undefined), "stability.crashes"), "regressed");
  const panicked = result({ startup: { ...result().startup, stability: { crashes: 0, panics: 2, hangs: 0 } } });
  assert.equal(statusOf(compare(panicked, undefined), "stability.panics"), "regressed");
  const leaky = result({ interaction: { ...result().interaction, memory: { growth: { rss_mb: -45, footprint_mb: 51 } } } });
  // RSS fell while the footprint grew: the footprint decides.
  assert.equal(statusOf(compare(leaky, undefined), "memory.cycle_growth_footprint_mb"), "regressed");
  const soakLeak = result({ soak: { stability: { crashes: 0, panics: 0, hangs: 0 }, memory: { growth: { rss_mb: 10, footprint_mb: 80 } } } });
  assert.equal(statusOf(compare(soakLeak, undefined), "soak.growth_footprint_mb"), "regressed");
});

test("only the same machine's baseline can judge timings", () => {
  const mac = { model: "Mac15,8", cpu: "Apple M3 Max", memory_gb: 64 };
  assert.ok(sameMachine(mac, { ...mac }));
  assert.ok(!sameMachine(mac, { ...mac, memory_gb: 36 }));
  assert.ok(!sameMachine(mac, undefined));
});

test("the summary names each failed check", () => {
  const now = result({ soak: { duration_s: 60, iterations: 3, stability: { crashes: 1, panics: 0, hangs: 0 }, memory: {}, crash_details: [{ at_s: 12, reason: "killed by SIGSEGV" }] } });
  now.config = { cycles: 20 };
  now.comparison = { checks: compare(now, undefined) };
  const text = summarize(now);
  assert.match(text, /❌ stability\.crashes/);
  assert.match(text, /crash at 12s: killed by SIGSEGV/);
  assert.match(text, /No baseline yet/);
});

test("command-line options", () => {
  const defaults = parseBenchArgs([]);
  assert.equal(defaults.soakSecs, 900);
  assert.deepEqual([...defaults.only].sort(), ["interaction", "soak", "startup"]);
  assert.equal(parseBenchArgs(["--soak", "2m"]).soakSecs, 120);
  assert.deepEqual(
    (({ startupRuns, cycles, soakSecs }) => ({ startupRuns, cycles, soakSecs }))(parseBenchArgs(["--quick"])),
    { startupRuns: 2, cycles: 5, soakSecs: 60 },
  );
  // --soak 0 skips the stage instead of launching a zero-length soak.
  assert.ok(!parseBenchArgs(["--soak", "0"]).only.has("soak"));
  assert.deepEqual([...parseBenchArgs(["--only", "startup"]).only], ["startup"]);
  assert.throws(() => parseBenchArgs(["--only", "release"]), /Unknown stage/);
  assert.throws(() => parseBenchArgs(["--soak", "soon"]), /duration/);
  assert.throws(() => parseBenchArgs(["--cycles", "0"]), /positive/);
  assert.throws(() => parseBenchArgs(["--soak"]), /needs a value/);
  assert.equal(parseDuration("1h"), 3600);
});

test("how a finished run counts as a crash", () => {
  const ended = (exit, done = { frame_timeouts: 0 }) => ({ exit, done });
  assert.equal(crashReason(ended({ code: 0 })), undefined);
  assert.equal(crashReason(ended({ code: null, signal: "SIGABRT" })), "killed by SIGABRT");
  assert.equal(crashReason(ended({ code: 101 })), "exited with code 101");
  // Quitting cleanly before the script finished still hides a problem.
  assert.equal(crashReason(ended({ code: 0 }, null)), "quit before the benchmark finished");
  // The runner's own kill reason wins over the signal it sent.
  assert.match(crashReason(ended({ code: null, signal: "SIGTERM", killedFor: "no benchmark event for 90s (hung)" })), /hung/);
});

test("soak growth skips the warm-up before the first 30 s sample", () => {
  // Measured on this Mac: +64 MB of first-use caches, then flat.
  const samples = [
    { label: "idle", rss_mb: 146, footprint_mb: 207.8 },
    { label: "soak_17", rss_mb: 176, footprint_mb: 272.3 },
    { label: "soak_34", rss_mb: 163.2, footprint_mb: 272.7 },
    { label: "soak_end", rss_mb: 169, footprint_mb: 281.8 },
  ];
  assert.deepEqual(soakGrowth(samples).growth, { rss_mb: -7, footprint_mb: 9.5 });
  // Too short for a sample after warm-up: measured from idle instead.
  assert.deepEqual(soakGrowth([samples[0], samples[3]]).growth, { rss_mb: 23, footprint_mb: 74 });
  // One sample is no growth at all.
  assert.equal(soakGrowth([samples[0]]).growth, undefined);
});
