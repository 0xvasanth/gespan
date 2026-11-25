import { motion, useReducedMotion, useScroll, useTransform } from "motion/react";
import { useRef, type ReactNode } from "react";
import { Shot, type ShotName } from "./Shot";
import "./macbook.css";

export interface LaptopPose {
  /** The lid's tilt back from upright, hinged at its bottom edge (degrees). */
  lid: number;
  /** The screen's scale: it grows past the laptop to fill the page. */
  scale: number;
  /** How far the screen has lifted (px, negative is up). */
  lift: number;
  /** The base's opacity: it falls away as the screen lifts out. */
  base: number;
}

const clamp = (p: number) => Math.min(1, Math.max(0, Number.isFinite(p) ? p : 1));
const easeInOut = (p: number) => (p < 0.5 ? 4 * p * p * p : 1 - (-2 * p + 2) ** 3 / 2);

/**
 * The laptop's pose for scroll progress 0…1. It starts with the lid leaning
 * back over the base; in the first half the lid swings upright, and in the
 * second the screen lifts out and grows while the base falls away.
 */
export function laptopPose(progress: number): LaptopPose {
  const p = clamp(progress);
  const open = easeInOut(Math.min(1, p / 0.5));
  const lift = easeInOut(Math.max(0, (p - 0.4) / 0.6));
  return {
    lid: 34 * (1 - open),
    scale: 0.94 + 0.06 * open + 0.16 * lift,
    lift: -40 * lift,
    base: 1 - lift,
  };
}

/** At rest, for reduced motion: open, upright, on its base. */
export const LAPTOP_AT_REST: LaptopPose = { lid: 0, scale: 1, lift: 0, base: 1 };

/**
 * A laptop showing a real Gespan screen (a shot, or a screen drawn in page
 * markup passed as children). As the page scrolls, the lid swings
 * up and the screen lifts out to fill the page (the stage keeps it in view
 * while that happens). Reduced motion shows it open and still.
 */
export function Macbook({ shot, children }: { shot?: ShotName; children?: ReactNode }) {
  const stage = useRef<HTMLDivElement>(null);
  const reduced = useReducedMotion();
  const { scrollYProgress } = useScroll({ target: stage, offset: ["start 75%", "end end"] });
  const lid = useTransform(scrollYProgress, (p) => laptopPose(p).lid);
  const scale = useTransform(scrollYProgress, (p) => laptopPose(p).scale);
  const lift = useTransform(scrollYProgress, (p) => laptopPose(p).lift);
  const base = useTransform(scrollYProgress, (p) => laptopPose(p).base);
  return (
    <div ref={stage} className={reduced ? "mb-stage still" : "mb-stage"} data-laptop="">
      <div className="mb-sticky">
        <motion.div className="mb-screen" style={reduced ? { rotateX: 0, scale: 1, y: 0 } : { rotateX: lid, scale, y: lift }}>
          <div className="mb-bezel">
            <span className="mb-camera" aria-hidden="true" />
            {shot ? <Shot name={shot} beam /> : <div className="mb-app">{children}</div>}
          </div>
        </motion.div>
        <motion.div className="mb-base" aria-hidden="true" style={reduced ? { opacity: 1 } : { opacity: base }}>
          <span className="mb-notch" />
        </motion.div>
      </div>
    </div>
  );
}
