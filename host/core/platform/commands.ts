// The desktop app these modules came from ran these commands in its Tauri
// backend. The host has no such backend: it configures its own child backend
// and never reaches these calls, so one that is reached fails loudly.
export type UnlistenFn = () => void;

function unavailable(name: string): Error {
  return new Error(`${name} is not available on the Gespan host`);
}

export function invoke<T>(command: string, _args?: Record<string, unknown>): Promise<T> {
  return Promise.reject(unavailable(command));
}

export function listen<T>(
  event: string,
  _handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  return Promise.reject(unavailable(event));
}

export function convertFileSrc(path: string, _protocol?: string): string {
  throw unavailable(`Opening ${path}`);
}
