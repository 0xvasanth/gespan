#!/usr/bin/env python3
"""End-to-end check of the Kiro provider in Gespan Native (macOS), with no AI.

Launches the built Gespan Native Dev app on a throwaway data dir and project,
points its Kiro provider at a deterministic ACP stub (fake-kiro-cli), then:

  1. presses Cmd+N for a new session (Kiro is preselected as the provider),
  2. sends a prompt, waits for the reply, then sends a follow-up,
  3. asserts that
     - the stub was launched as `kiro-cli acp --agent-engine=v3` and saw
       initialize -> one session/new (in the project) -> both prompts,
     - the follow-up reused the warm Kiro process (no restart, no session/load),
     - Gespan saved a session with harness "kiro", the stub's session id and
       both replies,
     - (optional, --ocr) the reply is visible in the app window.

Only keystrokes are sent (no mouse clicks), so overlays from other apps
cannot swallow input. Every other provider is pointed at /usr/bin/false, so
the run can never reach a real model.

Exit code 0 = pass, 1 = assertion failed, 2 = environment problem.

See README.md in this folder for requirements and examples.
"""
import argparse
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
DEFAULT_APP = REPO / "crates/gespan-gpui/target/native-dev/Gespan Native Dev.app"
STUB = HERE / "fake-kiro-cli"
STUB_REPLY = "GESPAN KIRO STUB REPLY"
OTHER_PROVIDERS = ["codex", "claude", "cursor", "grok", "opencode", "antigravity",
                   "pi", "omp", "fx", "hermes", "devin"]


class Failed(Exception):
    pass


class EnvError(Exception):
    pass


def log(message):
    print(f"[kiro-e2e] {message}", flush=True)


def osascript(script):
    result = subprocess.run(["osascript", "-e", script], capture_output=True, text=True)
    if result.returncode != 0:
        raise EnvError(f"osascript failed: {result.stderr.strip()} "
                       "(grant Accessibility to this terminal in System Settings > Privacy & Security)")
    return result.stdout.strip()


def process_ref(pid):
    return f'(first application process whose unix id is {pid})'


def focus(pid):
    osascript(f'tell application "System Events" to set frontmost of {process_ref(pid)} to true')
    deadline = time.time() + 5
    while time.time() < deadline:
        front = osascript('tell application "System Events" to get unix id of '
                          '(first application process whose frontmost is true)')
        if front == str(pid):
            return
        time.sleep(0.2)
    raise EnvError("could not bring the Gespan test app to the front")


def keystroke(pid, text=None, key_code=None, command=False):
    # Re-focus before every keystroke so nothing typed can land in another app.
    focus(pid)
    using = " using {command down}" if command else ""
    if key_code is not None:
        osascript(f'tell application "System Events" to key code {key_code}{using}')
    else:
        osascript(f'tell application "System Events" to keystroke {json.dumps(text)}{using}')


def wait_for(what, check, timeout, interval=0.5):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        try:
            value = check()
        except (sqlite3.Error, OSError, json.JSONDecodeError) as error:
            value, last = None, error
        if value:
            return value
        time.sleep(interval)
    raise Failed(f"timed out after {timeout}s waiting for {what}" + (f" (last error: {last})" if last else ""))


def window_count(pid):
    try:
        return int(osascript(f'tell application "System Events" to count windows of {process_ref(pid)}') or 0)
    except EnvError as error:
        if "Invalid index" in str(error):
            return 0  # not registered with System Events yet
        raise


def seed_preferences(data):
    """Make Kiro the provider for new sessions, as if it was the last one used.

    Written as the legacy settings file, which Gespan imports on first launch.
    """
    workspace = json.dumps({"preferences": {"last_provider": "kiro"}})
    (data / "gespan.db.gpui.json").write_text(json.dumps({"workspace": workspace}))


def saved_sessions(data):
    path = data / "gespan.db"
    if not path.exists():
        return []
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    try:
        rows = db.execute("SELECT harness, provider_session_id, cwd, blocks_json FROM sessions "
                          "WHERE has_user_message = 1").fetchall()
    finally:
        db.close()
    return [dict(harness=h, provider_session_id=p, cwd=c, blocks=json.loads(b)) for h, p, c, b in rows]


def stub_frames(stub_log):
    if not stub_log.exists():
        return []
    return [json.loads(line) for line in stub_log.read_text().splitlines() if line.strip()]


def replied_session(data, prompt):
    """The saved session whose last user block is `prompt` and that has a reply after it."""
    for session in saved_sessions(data):
        blocks = session["blocks"]
        asked = [i for i, b in enumerate(blocks) if b.get("role") == "user" and b.get("text") == prompt]
        if asked and any(b.get("role") == "assistant" and (b.get("text") or "").strip()
                         for b in blocks[asked[-1] + 1:]):
            return session
    return None


def check_wire(frames, project, prompts):
    """What the stub saw: launch flags, the ACP handshake, prompts, and warm reuse."""
    launches = [f["argv"] for f in frames if "argv" in f]
    if ["acp", "--agent-engine=v3"] not in launches:
        raise Failed(f"Kiro was not launched as `kiro-cli acp --agent-engine=v3`; launches: {launches}")
    # Only the processes that served the test project; the app also probes
    # Kiro's model catalog from other folders.
    news = [f for f in frames if f.get("method") == "session/new"
            and os.path.realpath(f["params"]["cwd"]) == os.path.realpath(project)]
    if len(news) != 1:
        raise Failed(f"expected one session/new in the test project, saw {len(news)}")
    sent = [" ".join(b.get("text", "") for b in f["params"]["prompt"])
            for f in frames if f.get("method") == "session/prompt"]
    for prompt in prompts:
        if not any(prompt in text for text in sent):
            raise Failed(f"no session/prompt carried {prompt!r}; prompts sent: {sent}")
    turn_pids = {f["pid"] for f in frames if f.get("method") == "session/prompt"}
    if len(turn_pids) != 1:
        raise Failed(f"follow-up did not reuse the warm Kiro process: turns ran in {len(turn_pids)} processes")
    if any(f.get("method") == "session/load" for f in frames if f["pid"] in turn_pids):
        raise Failed("follow-up reloaded the Kiro session instead of reusing it")


def check_session(session, prompts):
    """What Gespan saved for the conversation it rendered."""
    if session["harness"] != "kiro":
        raise Failed(f"session ran on {session['harness']!r}, not kiro")
    if not (session["provider_session_id"] or "").startswith("stub-session-"):
        raise Failed(f"session id {session['provider_session_id']!r} did not come from Kiro")
    replies = [b.get("text") or "" for b in session["blocks"] if b.get("role") == "assistant"]
    for prompt in prompts:
        expected = f"{STUB_REPLY}: {prompt}"
        if not any(expected in reply for reply in replies):
            raise Failed(f"no assistant reply contains {expected!r}; replies: {replies}")
    log(f"replies saved: {[r.strip() for r in replies]}")


def ocr_window(pid, out):
    tool = out / "uitool"
    subprocess.run(["swiftc", "-O", "-o", str(tool), str(HERE / "uitool.swift")], check=True)
    window = subprocess.run([str(tool), "window", str(pid)], capture_output=True, text=True).stdout.strip()
    shot = out / "window.png"
    if not window or subprocess.run(["screencapture", "-x", "-o", f"-l{window}", str(shot)]).returncode != 0:
        raise EnvError("window capture failed (grant Screen Recording to this terminal, or drop --ocr)")
    return subprocess.run([str(tool), "ocr", str(shot)], capture_output=True, text=True, check=True).stdout


def run(args):
    app = Path(args.app)
    executable = app / "Contents/MacOS/gespan-gpui"
    if not executable.exists():
        raise EnvError(f"{executable} not found; build it first (see README.md)")
    kiro = Path(args.kiro_binary)

    work = Path(tempfile.mkdtemp(prefix="gespan-kiro-e2e-"))
    data, project, stub_log = work / "data", work / "project", work / "kiro-stub.jsonl"
    data.mkdir()
    project.mkdir()
    subprocess.run(["git", "init", "-q"], cwd=project, check=True)
    (project / "README.md").write_text("Gespan Kiro end-to-end fixture\n")
    seed_preferences(data)
    log(f"work dir: {work}")

    env = dict(os.environ)
    env.update(GESPAN_NATIVE_DATA_DIR=str(data), GESPAN_KIRO_BINARY=str(kiro), KIRO_STUB_LOG=str(stub_log))
    for kind in OTHER_PROVIDERS:
        env[f"GESPAN_{kind.upper()}_BINARY"] = "/usr/bin/false"

    app_log = open(work / "app.log", "w")
    proc = subprocess.Popen([str(executable), "--cwd", str(project)], env=env,
                            stdout=app_log, stderr=subprocess.STDOUT)
    try:
        wait_for("the Gespan window", lambda: proc.poll() is None and window_count(proc.pid) > 0, 60)
        time.sleep(args.settle)
        log("app is up; starting a new session")
        keystroke(proc.pid, "n", command=True)
        time.sleep(args.settle)
        prompts = [f"kiro e2e ping {os.getpid()}", f"kiro e2e follow-up {os.getpid()}"]
        session = None
        for turn, prompt in enumerate(prompts, 1):
            keystroke(proc.pid, prompt)
            keystroke(proc.pid, key_code=36)  # Return
            log(f"turn {turn}: sent {prompt!r}")
            started = time.monotonic()
            session = wait_for(f"Gespan to save the reply to turn {turn}",
                               lambda: replied_session(data, prompt), args.timeout)
            log(f"turn {turn}: reply saved after {time.monotonic() - started:.1f}s")
            time.sleep(1)  # let the finished agent park before the next turn
        check_wire(stub_frames(stub_log), project, prompts)
        check_session(session, prompts)

        if args.ocr:
            text = ocr_window(proc.pid, work)
            if STUB_REPLY not in text:
                raise Failed(f"reply not visible in the window; OCR saw: {text[:400]!r}")
            log("reply is visible in the window (OCR)")
    finally:
        # Kiro children exit on their own once the app closes their stdin.
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        app_log.close()
        if not args.keep:
            subprocess.run(["rm", "-rf", str(work)])
        else:
            log(f"kept {work}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--app", default=str(DEFAULT_APP), help="path to Gespan Native Dev.app")
    parser.add_argument("--kiro-binary", default=str(STUB),
                        help="binary Gespan launches as kiro-cli (default: the deterministic stub)")
    parser.add_argument("--timeout", type=float, default=45, help="seconds to wait for the reply")
    parser.add_argument("--settle", type=float, default=2, help="pause after launch and Cmd+N")
    parser.add_argument("--ocr", action="store_true", help="also OCR the window (needs Screen Recording)")
    parser.add_argument("--keep", action="store_true", help="keep the temp data dir, logs and stub trace")
    args = parser.parse_args()
    if sys.platform != "darwin":
        print("This test drives the macOS app and runs on macOS only.", file=sys.stderr)
        return 2
    try:
        run(args)
    except Failed as error:
        log(f"FAIL: {error}")
        return 1
    except EnvError as error:
        log(f"ERROR: {error}")
        return 2
    log("PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
