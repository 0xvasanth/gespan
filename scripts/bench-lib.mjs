// Pure parts of the dev benchmark (scripts/bench.mjs): reading the app's
// stderr, summarizing runs and comparing them with the stored baseline.

export const BENCH_PREFIX = "GESPAN_BENCH ";

/** Regression limits ("balanced"). A check fails only when it is over both
 * the relative limit and the absolute floor, so tiny numbers don't flap. */
export const THRESHOLDS = {
  startupPct: 25,
  startupFloorMs: 100,
  frameP95Pct: 30,
  frameP95FloorMs: 2,
  latencyP95Pct: 30,
  latencyP95FloorMs: 5,
  longFrameMs: 250,
  idleRssPct: 20,
  idleRssFloorMb: 20,
  growthMb: 50,
};

/** GPUI prints `frame duration: <Duration as Debug>` with ZED_MEASUREMENTS=1. */
export function parseDurationMs(text) {
  const match = /^([\d.]+)(ns|µs|us|ms|s)$/.exec(text.trim());
  if (!match) return undefined;
  const value = Number(match[1]);
  const scale = { ns: 1e-6, "µs": 1e-3, us: 1e-3, ms: 1, s: 1000 }[match[2]];
  return value * scale;
}

/** One stderr line: a bench event, a frame, a panic, or nothing of ours. */
export function parseLine(line) {
  if (line.startsWith(BENCH_PREFIX)) {
    try {
      return { kind: "event", event: JSON.parse(line.slice(BENCH_PREFIX.length)) };
    } catch {
      return { kind: "other" };
    }
  }
  const frame = /^frame duration: (\S+)\s*$/.exec(line);
  if (frame) {
    const ms = parseDurationMs(frame[1]);
    return ms === undefined ? { kind: "other" } : { kind: "frame", ms };
  }
  if (/panicked at/.test(line)) return { kind: "panic", line };
  return { kind: "other" };
}

/** Nearest-rank percentile of an unsorted list. */
export function percentile(values, p) {
  if (!values.length) return undefined;
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.max(1, Math.ceil((p / 100) * sorted.length));
  return sorted[rank - 1];
}

export function median(values) {
  return percentile(values, 50);
}

export function stats(values) {
  if (!values.length) return { count: 0 };
  return {
    count: values.length,
    p50: round(percentile(values, 50)),
    p95: round(percentile(values, 95)),
    max: round(Math.max(...values)),
  };
}

export function round(value, digits = 2) {
  if (value === undefined || Number.isNaN(value)) return undefined;
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

/**
 * Folds one run's timestamped stderr lines into what the app reported.
 * `lines` are `{ at, text }` with `at` in wall-clock ms.
 */
export function collectRun(lines, spawnedAt) {
  const run = {
    events: [],
    frames: [],
    phases: {},
    steps: [],
    memory: [],
    layout: [],
    panics: [],
    firstFrameAt: undefined,
    done: undefined,
  };
  let phase;
  for (const { at, text } of lines) {
    const parsed = parseLine(text);
    if (parsed.kind === "frame") {
      run.firstFrameAt ??= at;
      run.frames.push({ ms: parsed.ms, phase });
      if (phase) (run.phases[phase] ??= { frames: [], latency: [], inactive: 0 }).frames.push(parsed.ms);
    } else if (parsed.kind === "panic") {
      run.panics.push(parsed.line);
    } else if (parsed.kind === "event") {
      const event = parsed.event;
      run.events.push(event);
      if (event.event === "phase_start") phase = event.phase;
      else if (event.event === "phase_end") phase = undefined;
      else if (event.event === "step") {
        const bucket = (run.phases[event.phase] ??= { frames: [], latency: [], inactive: 0 });
        bucket.latency.push(event.latency_ms);
        if (!event.active) bucket.inactive++;
        run.steps.push(event);
      } else if (event.event === "memory") run.memory.push(event);
      else if (event.event === "layout") run.layout.push(event);
      else if (event.event === "done") run.done = event;
    }
  }
  const find = (name) => run.events.find((e) => e.event === name);
  const since = (at) => (at === undefined ? undefined : round(at - spawnedAt, 1));
  run.startup = {
    app_run_ms: since(find("app_run")?.wall_ms),
    first_frame_ms: since(run.firstFrameAt),
    interactive_ms: since(find("interactive")?.wall_ms),
  };
  return run;
}

/** Frame and latency stats for each measured phase. */
export function phaseStats(run, names) {
  const out = {};
  for (const name of names) {
    const bucket = run.phases[name] ?? { frames: [], latency: [], inactive: 0 };
    out[name] = {
      frame_ms: stats(bucket.frames),
      latency_ms: stats(bucket.latency),
      long_frames: bucket.frames.filter((ms) => ms > THRESHOLDS.longFrameMs).length,
      inactive_steps: bucket.inactive,
    };
  }
  return out;
}

export function memoryByLabel(run, label) {
  return run.memory.find((m) => m.label === label);
}

/** RSS and footprint growth from before the open/close cycles to after. */
export function memoryGrowth(run) {
  const before = memoryByLabel(run, "before_cycles");
  const after = memoryByLabel(run, "after_cycles");
  if (!before || !after) return undefined;
  return {
    rss_mb: round(after.rss_mb - before.rss_mb, 1),
    footprint_mb: round(after.footprint_mb - before.footprint_mb, 1),
  };
}

/**
 * Soak growth starts at the first sample taken after 30 s of actions: the
 * first terminal, splits and glyph caches are warm-up, not a leak.
 */
export function soakGrowth(samples) {
  const start = samples.find((m) => /^soak_\d+$/.test(m.label)) ?? samples[0];
  const end = samples.at(-1);
  if (!start || !end || start === end) return { start, end, growth: undefined };
  return {
    start, end,
    growth: { rss_mb: round(end.rss_mb - start.rss_mb, 1), footprint_mb: round(end.footprint_mb - start.footprint_mb, 1) },
  };
}

/** `15m`, `90s`, `1h`, or plain seconds. */
export function parseDuration(text) {
  const match = /^(\d+(?:\.\d+)?)(s|m|h)?$/.exec(String(text).trim());
  if (!match) throw new Error(`Cannot read the duration "${text}"; use 90s, 15m or 1h.`);
  return Math.round(Number(match[1]) * { s: 1, m: 60, h: 3600 }[match[2] ?? "s"]);
}

function check(metric, baseline, current, { pct, floor, unit }) {
  if (baseline === undefined || current === undefined) {
    return { metric, baseline, current, status: "skipped", limit: "no baseline value" };
  }
  const allowed = Math.max(baseline * (1 + pct / 100), baseline + floor);
  return {
    metric,
    baseline,
    current,
    limit: `+${pct}% (min +${floor}${unit})`,
    change_pct: baseline ? round(((current - baseline) / baseline) * 100, 1) : undefined,
    status: current > allowed ? "regressed" : "ok",
  };
}

/**
 * Every check of a result against a baseline. Absolute checks (crashes,
 * panics, hangs, leak growth, long frames) apply with or without a baseline.
 */
export function compare(result, baseline, t = THRESHOLDS) {
  const checks = [];
  if (result.startup && baseline?.startup) {
    for (const key of ["first_frame_ms", "interactive_ms"]) {
      checks.push(check(`startup.${key}`, baseline.startup.median?.[key], result.startup.median?.[key],
        { pct: t.startupPct, floor: t.startupFloorMs, unit: "ms" }));
    }
    checks.push(check("memory.idle_rss_mb", baseline.startup.median?.idle_rss_mb, result.startup.median?.idle_rss_mb,
      { pct: t.idleRssPct, floor: t.idleRssFloorMb, unit: "MB" }));
  }
  for (const [phase, now] of Object.entries(result.interaction?.phases ?? {})) {
    const before = baseline?.interaction?.phases?.[phase];
    if (before) {
      checks.push(check(`${phase}.frame_p95_ms`, before.frame_ms?.p95, now.frame_ms?.p95,
        { pct: t.frameP95Pct, floor: t.frameP95FloorMs, unit: "ms" }));
      checks.push(check(`${phase}.latency_p95_ms`, before.latency_ms?.p95, now.latency_ms?.p95,
        { pct: t.latencyP95Pct, floor: t.latencyP95FloorMs, unit: "ms" }));
    }
    // A frame over the limit is a regression unless the baseline had one too.
    const newLong = now.frame_ms?.max > t.longFrameMs && !(before?.frame_ms?.max > t.longFrameMs);
    checks.push({
      metric: `${phase}.max_frame_ms`,
      baseline: before?.frame_ms?.max,
      current: now.frame_ms?.max,
      limit: `<= ${t.longFrameMs}ms`,
      status: newLong ? "regressed" : "ok",
    });
  }
  const growth = result.interaction?.memory?.growth;
  if (growth) {
    // Footprint, not RSS: macOS compresses and purges resident pages, so
    // RSS can drop by tens of MB while the app holds more memory.
    checks.push({
      metric: "memory.cycle_growth_footprint_mb",
      baseline: baseline?.interaction?.memory?.growth?.footprint_mb,
      current: growth.footprint_mb,
      limit: `<= ${t.growthMb}MB`,
      status: growth.footprint_mb > t.growthMb ? "regressed" : "ok",
    });
  }
  const stability = [result.interaction?.stability, result.startup?.stability, result.soak?.stability].filter(Boolean);
  for (const key of ["crashes", "panics", "hangs"]) {
    const total = stability.reduce((sum, s) => sum + (s[key] ?? 0), 0);
    checks.push({ metric: `stability.${key}`, current: total, limit: "0", status: total > 0 ? "regressed" : "ok" });
  }
  if (result.soak?.memory?.growth) {
    checks.push({
      metric: "soak.growth_footprint_mb",
      baseline: baseline?.soak?.memory?.growth?.footprint_mb,
      current: result.soak.memory.growth.footprint_mb,
      limit: `<= ${t.growthMb}MB`,
      status: result.soak.memory.growth.footprint_mb > t.growthMb ? "regressed" : "ok",
    });
  }
  return checks;
}

/** A baseline from another machine can't judge timings: they only warn. */
export function sameMachine(a, b) {
  return Boolean(a && b && a.model === b.model && a.cpu === b.cpu && a.memory_gb === b.memory_gb);
}

const fmt = (value, unit = "") => (value === undefined ? "–" : `${value}${unit}`);

export function summarize(result) {
  const lines = [];
  const m = result.machine ?? {};
  lines.push(`# Gespan dev benchmark — ${result.created_at}`);
  lines.push("");
  lines.push(`Commit ${result.git?.commit ?? "?"}${result.git?.dirty ? " (dirty)" : ""} · ${m.model ?? "?"} · ${m.cpu ?? "?"} · ${m.memory_gb ?? "?"} GB · load ${result.load?.start?.join(" ") ?? "?"} → ${result.load?.end?.join(" ") ?? "?"}`);
  lines.push("");
  if (result.startup) {
    const s = result.startup.median;
    lines.push(`## Startup (median of ${result.startup.runs?.length ?? "?"} launches)`);
    lines.push("");
    lines.push("| launch → app.run | → first frame | → interactive | idle RSS | idle footprint |");
    lines.push("|---|---|---|---|---|");
    lines.push(`| ${fmt(s.app_run_ms, " ms")} | ${fmt(s.first_frame_ms, " ms")} | ${fmt(s.interactive_ms, " ms")} | ${fmt(s.idle_rss_mb, " MB")} | ${fmt(s.idle_footprint_mb, " MB")} |`);
    lines.push("");
  }
  if (result.interaction) {
    lines.push("## Frames and input latency");
    lines.push("");
    lines.push("| phase | frames | frame p50 | frame p95 | frame max | input→paint p50 | p95 | max |");
    lines.push("|---|---|---|---|---|---|---|---|");
    for (const [phase, p] of Object.entries(result.interaction.phases)) {
      lines.push(`| ${phase} | ${p.frame_ms.count} | ${fmt(p.frame_ms.p50, " ms")} | ${fmt(p.frame_ms.p95, " ms")} | ${fmt(p.frame_ms.max, " ms")} | ${fmt(p.latency_ms.p50, " ms")} | ${fmt(p.latency_ms.p95, " ms")} | ${fmt(p.latency_ms.max, " ms")} |`);
    }
    lines.push("");
    const mem = result.interaction.memory;
    lines.push(`## Memory over ${result.config.cycles} open/close cycles`);
    lines.push("");
    lines.push(`Footprint ${fmt(mem.before?.footprint_mb, " MB")} → ${fmt(mem.after?.footprint_mb, " MB")} (growth ${fmt(mem.growth?.footprint_mb, " MB")}); RSS ${fmt(mem.before?.rss_mb, " MB")} → ${fmt(mem.after?.rss_mb, " MB")}.`);
    lines.push("");
  }
  if (result.soak) {
    const s = result.soak;
    lines.push(`## Soak (${s.duration_s}s)`);
    lines.push("");
    lines.push(`${s.iterations} action rounds, ${s.stability.crashes} crashes, ${s.stability.panics} panics, ${s.stability.hangs} hang reports. Footprint ${fmt(s.memory.start?.footprint_mb, " MB")} → ${fmt(s.memory.end?.footprint_mb, " MB")} (growth ${fmt(s.memory.growth?.footprint_mb, " MB")}); RSS ${fmt(s.memory.start?.rss_mb, " MB")} → ${fmt(s.memory.end?.rss_mb, " MB")}.`);
    for (const crash of s.crash_details ?? []) lines.push(`- crash at ${crash.at_s}s: ${crash.reason}`);
    lines.push("");
  }
  const checks = result.comparison?.checks ?? [];
  const bad = checks.filter((c) => c.status === "regressed");
  lines.push("## Against the baseline");
  lines.push("");
  if (!result.comparison?.baseline_created_at) lines.push("No baseline yet: only absolute checks ran. Save one with `--save-baseline`.");
  else lines.push(`Baseline from ${result.comparison.baseline_created_at} (${result.comparison.same_machine ? "this machine" : "another machine: timing checks only warn"}).`);
  lines.push("");
  if (bad.length) {
    lines.push("| check | baseline | now | limit |");
    lines.push("|---|---|---|---|");
    for (const c of bad) lines.push(`| ❌ ${c.metric} | ${fmt(c.baseline)} | ${fmt(c.current)} | ${c.limit} |`);
  } else {
    lines.push(`All ${checks.length} checks passed.`);
  }
  for (const warning of result.warnings ?? []) lines.push(`\n> ${warning}`);
  return lines.join("\n") + "\n";
}
