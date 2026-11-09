// Windows keeps a file open for a moment after the process using it exits, and
// antivirus scans freshly written executables, so archiving the packaged
// node.exe right after its smoke test can fail with a sharing violation.
const LOCKED = /being used by another process|sharing violation/i;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Runs `run`, retrying only while the failure is a file still being in use. */
export async function retryWhileLocked(
  run,
  { attempts = 5, delayMs = 1000, wait = sleep } = {},
) {
  for (let attempt = 1; ; attempt++) {
    try {
      return run();
    } catch (error) {
      const text = `${error?.message ?? ""}\n${error?.stderr ?? ""}`;
      if (attempt >= attempts || !LOCKED.test(text)) throw error;
      await wait(delayMs * attempt);
    }
  }
}
