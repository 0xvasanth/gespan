import { createContext } from "react";
import type { HoldPhase } from "./hold";

/** The app-wide hold, for surfaces deep in the tree such as the message queue. */
export const HoldPhaseContext = createContext<HoldPhase | null>(null);
