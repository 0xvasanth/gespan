import { lazy, Suspense, useState, type ComponentType } from "react";

/** Keep optional surfaces out of the composer's startup dependency graph. */
export function lazySurface<Props extends object>(
  load: () => Promise<{ default: ComponentType<Props> }>,
  { suspense = true }: { suspense?: boolean } = {},
) {
  let pending: ReturnType<typeof load> | undefined;
  let loaded: ComponentType<Props> | undefined;
  const preload = () => {
    pending ??= load()
      .then((module) => {
        loaded = module.default;
        return module;
      })
      .catch((error: unknown) => {
        pending = undefined;
        throw error;
      });
    return pending;
  };
  const Surface = lazy(preload);
  function LazySurface(props: Props) {
    // A prefetched module is ready to render without a first lazy suspension.
    // Keep that choice for this mount so later warmups cannot reset its state.
    const [Component] = useState(() => loaded ?? Surface);
    // Navigation uses the app's existing boundary so a transition keeps the
    // current screen visible until the destination has loaded.
    if (!suspense) return <Component {...props} />;
    return (
      <Suspense fallback={null}>
        <Component {...props} />
      </Suspense>
    );
  }
  return Object.assign(LazySurface, { preload });
}
