import { createContext, useContext } from "react";
import type { PriorityScheme } from "../model/board";
import { DEFAULT_PRIORITY_SCHEME } from "../model/boardSettings";

/** The open board's priority names and hidden levels; the defaults outside a board. */
export const PriorityContext = createContext<PriorityScheme>(
  DEFAULT_PRIORITY_SCHEME,
);

export function usePriorityScheme(): PriorityScheme {
  return useContext(PriorityContext);
}
