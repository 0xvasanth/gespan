import { motion, useReducedMotion, useScroll, useTransform } from "motion/react";
import { useRef } from "react";
import { Shot, type ShotName } from "./Shot";

export interface Pose {
  rotateX: number;
  scale: number;
  y: number;
  opacity: number;
}

/** The hero screenshot's pose for scroll progress 0…1: tilted back and low, then upright and full size. */
export function heroPose(progress: number): Pose {
  const p = Math.min(1, Math.max(0, Number.isFinite(progress) ? progress : 1));
  const ease = 1 - (1 - p) ** 3;
  return { rotateX: 26 * (1 - ease), scale: 0.86 + 0.14 * ease, y: 70 * (1 - ease), opacity: 0.55 + 0.45 * ease };
}

/** A section screenshot's pose: rises and settles as it comes into view. */
export function risePose(progress: number): Pose {
  const p = Math.min(1, Math.max(0, Number.isFinite(progress) ? progress : 1));
  const ease = 1 - (1 - p) ** 3;
  return { rotateX: 12 * (1 - ease), scale: 0.92 + 0.08 * ease, y: 90 * (1 - ease), opacity: ease };
}

const UPRIGHT: Pose = { rotateX: 0, scale: 1, y: 0, opacity: 1 };

/**
 * A product shot that pops up as you scroll: tilted back in 3D, it lifts,
 * straightens and grows into place. Visitors who prefer reduced motion get the
 * shot at rest.
 */
export function ScrollShot({ name, className, variant = "rise", beam = false }: { name: ShotName; className?: string; variant?: "hero" | "rise"; beam?: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const reduced = useReducedMotion();
  const { scrollYProgress } = useScroll({
    target: ref,
    // The hero is already on screen: it settles over the first stretch of scrolling.
    offset: variant === "hero" ? ["start 105%", "start 30%"] : ["start 100%", "start 45%"],
  });
  const pose = variant === "hero" ? heroPose : risePose;
  const rotateX = useTransform(scrollYProgress, (p) => (reduced ? 0 : pose(p).rotateX));
  const scale = useTransform(scrollYProgress, (p) => (reduced ? 1 : pose(p).scale));
  const y = useTransform(scrollYProgress, (p) => (reduced ? 0 : pose(p).y));
  const opacity = useTransform(scrollYProgress, (p) => (reduced ? 1 : pose(p).opacity));
  return (
    <div ref={ref} style={{ perspective: "1600px" }} data-scroll-shot={variant}>
      <motion.div style={reduced ? UPRIGHT : { rotateX, scale, y, opacity, transformOrigin: "50% 0%" }}>
        <Shot name={name} className={className} beam={beam} />
      </motion.div>
    </div>
  );
}
