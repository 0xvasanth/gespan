import { extraKey, type LinkExtra } from "./linkStatus";

/** A PR or worktree link whose status is read with `gh` and `git`. */
export type StatusLink = { kind: "pr" | "worktree"; reference: string };

/** At most this many links are read at once (each read is a few processes). */
export const LINK_STATUS_CONCURRENCY = 4;
/** A link read this recently answers from its last result instead. */
export const LINK_STATUS_FRESH_MS = 5000;

/** Last result per link key, and when it was read. */
export type LinkStatusCache = Map<string, { at: number; value: LinkExtra }>;

/**
 * Runs `task` over `items` with at most `limit` running at once; results keep
 * the input order. A task that throws rejects the whole call, like `Promise.all`.
 */
export async function mapLimited<T, R>(
  items: readonly T[],
  limit: number,
  task: (item: T) => Promise<R>,
): Promise<R[]> {
  const out = new Array<R>(items.length);
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const at = next++;
      out[at] = await task(items[at]!);
    }
  };
  await Promise.all(
    Array.from({ length: Math.min(Math.max(1, limit), items.length) }, worker),
  );
  return out;
}

/**
 * Reads the status of each link, keyed by `extraKey`: a link whose last
 * result is newer than `freshMs` answers from `cache`; the rest are read
 * through `read`, at most `limit` at a time, and cached.
 */
export async function lookupLinkStatuses(
  links: readonly StatusLink[],
  {
    read,
    cache,
    limit = LINK_STATUS_CONCURRENCY,
    freshMs = LINK_STATUS_FRESH_MS,
    now = Date.now,
  }: {
    read: (link: StatusLink) => Promise<LinkExtra>;
    cache: LinkStatusCache;
    limit?: number;
    freshMs?: number;
    now?: () => number;
  },
): Promise<Record<string, LinkExtra>> {
  const out: Record<string, LinkExtra> = {};
  const stale: StatusLink[] = [];
  const seen = new Set<string>();
  for (const link of links) {
    const key = extraKey(link);
    if (seen.has(key)) continue;
    seen.add(key);
    const hit = cache.get(key);
    if (hit && now() - hit.at < freshMs) out[key] = hit.value;
    else stale.push(link);
  }
  await mapLimited(stale, limit, async (link) => {
    const value = await read(link);
    const key = extraKey(link);
    cache.set(key, { at: now(), value });
    out[key] = value;
  });
  return out;
}
