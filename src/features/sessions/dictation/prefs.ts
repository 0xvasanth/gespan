export const DICTATION_MODEL_EN = "parakeet-tdt-0.6b-v2-en";
export const DICTATION_MODEL_MULTI = "parakeet-tdt-0.6b-v3-multi";
export const DICTATION_DEFAULT_MODEL = DICTATION_MODEL_EN;

const MODEL_KEY = "gespan.dictation.model";
const WORDS_KEY = "gespan.dictation.words";

export function loadDictationModel(): string {
  try {
    const raw = localStorage.getItem(MODEL_KEY);
    return raw === DICTATION_MODEL_MULTI ? raw : DICTATION_DEFAULT_MODEL;
  } catch {
    return DICTATION_DEFAULT_MODEL;
  }
}

export function saveDictationModel(id: string) {
  try {
    localStorage.setItem(MODEL_KEY, id);
  } catch {
    // private mode / quota
  }
}

/** One word or phrase per line; blank lines dropped. */
export function parseDictationWords(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

/** The custom words as typed, newlines included. */
export function loadDictationWordsText(): string {
  try {
    return localStorage.getItem(WORDS_KEY) ?? "";
  } catch {
    return "";
  }
}

export function loadDictationWords(): string[] {
  return parseDictationWords(loadDictationWordsText());
}

export function saveDictationWordsText(text: string) {
  try {
    localStorage.setItem(WORDS_KEY, text);
  } catch {
    // private mode / quota
  }
}

/** A rejected download that the user cancelled; not an error to show. */
export function isCancelledDownload(reason: unknown): boolean {
  return String(reason).toLowerCase().includes("cancel");
}

export function formatModelSize(bytes: number): string {
  return `${Math.round(bytes / 1_000_000)} MB`;
}
