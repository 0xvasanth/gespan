import { motion, useReducedMotion, useScroll, useTransform } from "motion/react";
import { useRef } from "react";
import { Shot, type ShotName } from "./Shot";

export interface Pose {
  rotateX: number;
  scale: number;
  y: number;
  opacity: number;
}

const clamp = (progress: number) => Math.min(1, Math.max(0, Number.isFinite(progress) ? progress : 1));
const easeOut = (p: number) => 1 - (1 - p) ** 3;

/**
 * The hero screenshot's pose for scroll progress 0…1: it lies tilted far back,
 * low and small, then pops up, straightening and growing to full size.
 */
export function heroPose(progress: number): Pose {
  const ease = easeOut(clamp(progress));
  return { rotateX: 38 * (1 - ease), scale: 0.78 + 0.22 * ease, y: 140 * (1 - ease), opacity: 0.7 + 0.3 * ease };
}

/**
 * A section screenshot's pose: tilted and lower as it enters, settling as it
 * comes up the screen. It is never faint, so it never reads as an empty box.
 */
export function risePose(progress: number): Pose {
  const ease = easeOut(clamp(progress));
  return { rotateX: 18 * (1 - ease), scale: 0.88 + 0.12 * ease, y: 70 * (1 - ease), opacity: 0.75 + 0.25 * ease };
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
    // The hero starts low on the first screen and pops up over the first
    // stretch of scrolling; a section shot settles as it climbs the screen.
    offset: variant === "hero" ? ["start 90%", "start 20%"] : ["start end", "start 40%"],
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
