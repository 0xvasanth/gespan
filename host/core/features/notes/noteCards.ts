/** Note chip shown in the composer and on the user turn in the thread. */
export type NoteCardMeta = {
  id: string;
  slug: string;
  title: string;
  sourceCwd?: string;
};

/** Composer chip: display fields plus the body injected into the harness prompt. */
export type NoteComposerCard = NoteCardMeta & {
  body: string;
};
