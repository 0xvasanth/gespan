import type { CSSProperties } from "react";
import { PROJECT_MASCOTS } from "../../projects/model/projectMascots";
import { PixelMascot } from "../../projects/ui/PixelMascot";
import { TAB_GROUP_COLORS } from "../../workspace/model/tabGroups";
import { playCue } from "../../settings/model/sounds";
type Look = { mascot: string; color: string };

type Props = {
  /** The mascot and color the user's first Pilot will have. */
  look: Look;
  /** Creates the first Pilot and opens it. */
  onCreate: () => void;
  /** Turns Pilots off, as Settings › Pilots › Show pilots does. */
  onHide: () => void;
};

/** Gap between the mascots lining up at the back. */
const CROWD_STEP = 17;

/** The rest of the mascots, each in its own color, to line up behind the pilot. */
export function pilotIntroCrowd(look: Look) {
  const colors = TAB_GROUP_COLORS.slice(1).filter(
    (color) => color !== look.color,
  );
  return PROJECT_MASCOTS.map((mascot) => mascot.name)
    .filter((name) => name !== look.mascot)
    .map((name, i) => ({ name, color: colors[i % colors.length] }));
}

/**
 * Stands in for the rail's Pilots while the user has none, inviting them to
 * make their first. The switch hides Pilots altogether.
 */
export function PilotIntroCard({ look, onCreate, onHide }: Props) {
  const crowd = pilotIntroCrowd(look);
  const middle = (crowd.length - 1) / 2;

  return (
    <section
      aria-label="Meet Pilots"
      data-pilot-intro
      className="mx-2 overflow-hidden rounded-lg bg-content/[0.03]"
    >
      <div className="flex items-center gap-2 pl-2.5 pr-2 pt-2">
        <h3 className="min-w-0 flex-1 truncate text-[13px] font-medium text-content">
          Meet Pilots
        </h3>
        <button
          type="button"
          role="switch"
          aria-checked
          aria-label="Show pilots"
          title="Hide pilots from the rail. Turn them back on in Settings › Pilots."
          onClick={() => {
            onHide();
            playCue("switch");
          }}
          className="relative h-4 w-7 shrink-0 rounded-full bg-accent transition-colors"
        >
          <span className="absolute left-3.5 top-0.5 size-3 rounded-full bg-white" />
        </button>
      </div>

      <div
        aria-hidden
        data-pilot-intro-stage
        className="pilot-intro relative h-12 overflow-hidden"
      >
        {crowd.map((mascot, i) => {
          const offset = (i - middle) * CROWD_STEP;
          // Fill in from the outside, alternating sides.
          const order =
            Math.min(i, crowd.length - 1 - i) * 2 + (i < middle ? 0 : 1);
          return (
            <div
              key={mascot.name}
              className="pilot-intro-march absolute bottom-4 left-1/2 opacity-45"
              style={
                {
                  marginLeft: offset - 7,
                  "--from": `${offset < 0 ? -140 : 140}px`,
                  "--delay": `${order * 70}ms`,
                } as CSSProperties
              }
            >
              <div className="pilot-intro-hop">
                <PixelMascot
                  name={mascot.name}
                  color={mascot.color}
                  className="size-3.5"
                />
              </div>
            </div>
          );
        })}

        <div className="absolute bottom-0.5 left-1/2 h-2 w-14 -translate-x-1/2 bg-[radial-gradient(closest-side,color-mix(in_srgb,var(--color-content)_14%,transparent),transparent)]" />

        {/* The user's pilot drops in front once the rest are in. */}
        <div className="pilot-intro-drop absolute bottom-1 left-1/2 -ml-4">
          <PixelMascot
            name={look.mascot}
            color={look.color}
            className="size-8"
          />
        </div>
      </div>

      <p className="px-2.5 pt-1.5 text-[11px] leading-snug text-content/55">
        Agents of your own that work across your projects and run habits on
        their own.
      </p>

      <div className="p-2">
        <button
          type="button"
          onClick={onCreate}
          className="h-7 w-full cursor-default rounded-md bg-content text-[12px] font-medium text-background-base hover:bg-content/85 active:scale-[0.98]"
        >
          Create pilot
        </button>
      </div>
    </section>
  );
}
