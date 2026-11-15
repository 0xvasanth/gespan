import { useSyncExternalStore, type AnchorHTMLAttributes, type MouseEvent } from "react";
import routes from "./routes.json";

export type Page = "home" | "features" | "pilots" | "download" | "changelog" | "docs";
export interface Route {
  path: string;
  page: Page;
  title: string;
}
export const ROUTES = routes as Route[];

/** The route for a pathname, ignoring a trailing slash or index.html; null when no page matches. */
export function matchRoute(pathname: string): Route | null {
  let path = pathname.replace(/\/index\.html$/, "/");
  if (path.length > 1) path = path.replace(/\/+$/, "");
  return ROUTES.find((route) => route.path === (path || "/")) ?? null;
}

const CHANGE = "gespan:navigate";
const subscribe = (notify: () => void) => {
  window.addEventListener("popstate", notify);
  window.addEventListener(CHANGE, notify);
  return () => {
    window.removeEventListener("popstate", notify);
    window.removeEventListener(CHANGE, notify);
  };
};

/** The current pathname, updated on navigation and Back/Forward. */
export function usePathname(): string {
  return useSyncExternalStore(subscribe, () => window.location.pathname, () => "/");
}

export function navigate(to: string) {
  if (to === window.location.pathname) return;
  window.history.pushState(null, "", to);
  window.dispatchEvent(new Event(CHANGE));
  window.scrollTo(0, 0);
}

/** A link to another page of the site. A modified click (new tab, new window) is left to the browser. */
export function Link({ to, onClick, children, ...rest }: { to: string } & AnchorHTMLAttributes<HTMLAnchorElement>) {
  const handle = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    navigate(to);
  };
  return (
    <a href={to} onClick={handle} {...rest}>
      {children}
    </a>
  );
}
