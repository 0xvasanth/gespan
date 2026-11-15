import { useEffect } from "react";

const TARGETS = "[data-reveal]:not([data-shown]), [data-stagger]:not([data-shown])";

/**
 * Marks [data-reveal] and [data-stagger] elements with data-shown as they
 * scroll into view, once each; src/styles/motion.css animates the change.
 * Without IntersectionObserver, or when the visitor prefers reduced motion,
 * everything is shown at once and nothing is ever hidden.
 */
export function useReveal(key: string) {
  useEffect(() => {
    const root = document.documentElement;
    const all = () => document.querySelectorAll<HTMLElement>(TARGETS);
    const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
    if (reduced || typeof IntersectionObserver === "undefined") {
      for (const el of all()) el.dataset.shown = "";
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          (entry.target as HTMLElement).dataset.shown = "";
          observer.unobserve(entry.target);
        }
      },
      { rootMargin: "0px 0px -8% 0px", threshold: 0.12 },
    );
    for (const el of all()) observer.observe(el);
    root.classList.add("reveal-ready");
    return () => observer.disconnect();
  }, [key]);
}
