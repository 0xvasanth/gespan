import type { AutomationScheduleKind } from "../../automations/model/automationSchedule";

export type HabitSchedule = {
  scheduleKind: AutomationScheduleKind;
  /** Minute past the hour, for hourly habits. */
  minute: number;
  /** "HH:MM", local time. */
  time: string;
  /** 0 is Sunday, for weekly habits. */
  dayOfWeek: number;
};

/**
 * Cards a Pilot puts in its chat beside its words, with the app CLI's
 * `chat.card`. A card names what it is about and the app shows its current
 * state when it is drawn, so a PR card posted while the PR was open says
 * "Merged" the next day.
 */
export type PilotCard =
  | {
      type: "pr";
      /** owner/name; the project's own repository when left out. */
      repo?: string;
      number: number;
      /** One line from the Pilot on why it matters. */
      note?: string;
    }
  | { type: "session"; sessionId: string; note?: string }
  | {
      /** Replies the user can send with a tap, after a question. */
      type: "choices";
      options: string[];
    }
  | {
      /** A habit the Pilot suggests; nothing runs until the user starts it. */
      type: "habit";
      name: string;
      instructions: string;
      schedule: HabitSchedule;
    };
