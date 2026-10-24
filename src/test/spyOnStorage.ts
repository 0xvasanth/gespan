import { onTestFinished, vi } from "vitest";

type StorageMethod = "getItem" | "setItem" | "removeItem" | "clear";

/**
 * `vi.spyOn(localStorage, ...)` that is undone when the test finishes.
 *
 * happy-dom's Storage is a Proxy that binds methods onto the instance and
 * ignores `delete` for them, so Vitest's restore (which deletes the spy when
 * the original lived on the prototype) leaves the mock in place for every
 * later test. Put the bound original back explicitly instead.
 */
export function spyOnStorage<M extends StorageMethod>(
  storage: Storage,
  method: M,
) {
  const original = storage[method];
  const spy = vi.spyOn(storage, method);
  onTestFinished(() => {
    Object.defineProperty(storage, method, {
      value: original,
      configurable: true,
      writable: true,
    });
  });
  return spy;
}
