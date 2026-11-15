import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Joins class names and lets a later Tailwind class override an earlier one (shadcn's helper). */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
