import { motion, useReducedMotion } from "motion/react";
import type { ReactNode } from "react";
import "./lamp.css";

const EASE = [0.2, 0.7, 0.2, 1] as const;

/** The lamp's light before and after it turns on: it widens and brightens once. */
export const LAMP = {
  off: { cone: { scaleX: 0.45, opacity: 0.4 }, line: { scaleX: 0.4, opacity: 0.6 } },
  on: { cone: { scaleX: 1, opacity: 1 }, line: { scaleX: 1, opacity: 1 } },
} as const;

/**
 * A strip light at the top that, as it scrolls into view, widens and throws a
 * cone of violet light down onto what's beneath. It turns on once; with
 * reduced motion it is simply on.
 */
export function Lamp({ children }: { children: ReactNode }) {
  const reduced = useReducedMotion();
  const turn = (part: "cone" | "line") =>
    reduced
      ? { initial: false as const, animate: LAMP.on[part] }
      : { initial: LAMP.off[part], whileInView: LAMP.on[part], viewport: { once: true, amount: 0.5 }, transition: { duration: 0.9, delay: 0.15, ease: EASE } };
  return (
    <div className="lm" data-lamp="">
      <div className="lm-light" aria-hidden="true">
        <motion.div className="lm-cone" {...turn("cone")} />
        <div className="lm-glow" />
        <motion.div className="lm-line" {...turn("line")} />
      </div>
      <div className="lm-content">{children}</div>
    </div>
  );
}
