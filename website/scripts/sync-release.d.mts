import type { Release } from "./release.mjs";
export const RELEASE_FILE: string;
export function fetchRelease(tag?: string, options?: { fetchImpl?: typeof fetch; token?: string }): Promise<Release>;
export function writeRelease(release: Release, file?: string): { previous: string | null; next: string; changed: boolean };
