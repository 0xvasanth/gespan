import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * The project was renamed to Gespan. This guard keeps the two earlier product
 * names from coming back anywhere in the repository: in file contents, in file
 * and folder names, in identifiers, in docs.
 *
 * The one exception is the README's credit to the project Gespan was forked
 * from, which may name it as a link to the original repository.
 *
 * The old names are assembled from pieces below so that this file does not
 * contain them itself.
 */
const root = join(__dirname, "..", "..");

const cat = (...parts: string[]) => parts.join("");
const FIRST_NAME = cat("mo", "no", "co", "de"); // the original product name
const FIRST_SITE = cat("use", "mo", "no"); // its website
const FIRST_WORD = cat("mo", "no"); // what its agent feature was called
const SECOND_NAME = cat("re", "ba", "se"); // the name used before Gespan

// ---------------------------------------------------------------------------
// Gespan is a fork, and the README says so. The credit must link to the
// original repository; a bare mention, or one in any other file, still fails.

const CREDITED_IN = "README.md";
const UPSTREAM_URL = `https://github.com/hardbeat920/${FIRST_NAME}`;
const CREDIT_LINK = new RegExp(
  String.raw`\[${FIRST_NAME}\]\(${UPSTREAM_URL.replaceAll(".", String.raw`\.`)}\)`,
  "gi",
);
const withoutCredit = (line: string, file: string) =>
  file === CREDITED_IN ? line.replace(CREDIT_LINK, "") : line;

// ---------------------------------------------------------------------------
// Words that legitimately contain the first name's stem: typography and
// ordinary English. Stripped before looking for the stem.

const FONT_FAMILY = new RegExp(
  [
    String.raw`\b[A-Z][A-Za-z]*(?: [A-Z][A-Za-z]*)* ?${cat("Mo", "no")}\b`, // Roboto Mono, IBM Plex Mono, SF Mono
    String.raw`\b[A-Z][a-z]+${cat("Mo", "no")}(?:-[A-Za-z]+)?\b`, // JetBrainsMono
    String.raw`SF${cat("Mo", "no")}-[A-Za-z]+`,
  ].join("|"),
  "g",
);
const TYPOGRAPHY = new RegExp(
  [
    "ui-monospace",
    "monospace",
    "font-mono",
    "monokai",
    "monochrome",
    "mono(?:repo|tonic|lith|graph|poli[a-z]*|poly|type|morph|tone)",
  ]
    .join("|")
    .replaceAll("mono", FIRST_WORD),
  "gi",
);
const stripTypography = (line: string) =>
  line.replace(FONT_FAMILY, "").replace(TYPOGRAPHY, "");

// ---------------------------------------------------------------------------
// The second name is also a git term (and hides inside "firebase"), so it is
// matched the way the rename matched it: at the start of a word or camelCase
// hump, never before a lowercase letter, and never in the code terms below.

const WORD_START = String.raw`(?:(?<![A-Za-z])|(?<=\\n))`;
const PRODUCT_USES = [
  new RegExp(`${WORD_START}${SECOND_NAME.toUpperCase()}`),
  new RegExp(`${WORD_START}${SECOND_NAME}(?![a-z])`),
  new RegExp(`${SECOND_NAME[0].toUpperCase()}${SECOND_NAME.slice(1)}(?![a-z])`),
];
const KEPT_EVERYWHERE = [
  new RegExp(`git[- ]${SECOND_NAME}`, "i"), // the git command
  new RegExp(`--${SECOND_NAME}(?=")`), // `gh pr merge --<flag>`
  // Moving a project folder re-points the data saved under the old path.
  new RegExp(`${SECOND_NAME}(?=(?:Path|Project|CiRepairs|SessionFolderSettings))`),
  new RegExp(`${SECOND_NAME}(?=_(?:path|project|pending_scroll))`),
];
// The merge-method picker offers GitHub's "squash", "rebase" and "merge".
const MERGE_METHOD = [
  new RegExp(`"${SECOND_NAME}"`),
  new RegExp(`${SECOND_NAME[0].toUpperCase()}${SECOND_NAME.slice(1)} and merge\\??`),
  new RegExp(`${SECOND_NAME}d individually`),
];
const KEPT_IN_FILE: Record<string, RegExp[]> = {
  "src/features/inbox/ui/InboxView.tsx": MERGE_METHOD,
  "crates/gespan-gpui/src/inbox/ui.rs": MERGE_METHOD,
  "crates/gespan-gpui/src/inbox.rs": [new RegExp(`"squash" \\| "${SECOND_NAME}"`)],
  "crates/gespan-core/src/inbox/github.rs": [
    new RegExp(`"${SECOND_NAME}"(?= => vec!\\["pr")`),
  ],
  "src-tauri/src/fs.rs": [new RegExp(`"${SECOND_NAME}"(?= => vec!\\["pr")`)],
  "src/features/inbox/model/githubTasks.ts": [
    new RegExp(`"squash" \\| "${SECOND_NAME}"`),
  ],
};

const MASK = "\u0000";
function withoutKeptUses(text: string, file: string): string {
  let out = text;
  for (const re of [...KEPT_EVERYWHERE, ...(KEPT_IN_FILE[file] ?? [])]) {
    out = out.replace(new RegExp(re.source, `${re.flags.replace("g", "")}g`), (m) =>
      MASK.repeat(m.length),
    );
  }
  return out;
}
// Masking only blanks text out, so a line that never spells the word in any
// case cannot match. Skipping those keeps the whole-repo scan inside the test
// timeout on slower CI runners.
const MAY_NAME = new RegExp(SECOND_NAME, "i");
function usesSecondName(line: string, file = ""): boolean {
  if (!MAY_NAME.test(line)) return false;
  const masked = withoutKeptUses(line, file);
  return PRODUCT_USES.some((re) => re.test(masked));
}

// ---------------------------------------------------------------------------

const BINARY = /\.(png|jpe?g|gif|ico|icns|woff2?|ttf|otf|mp4|webm|zip|gz|pdf|car|a|so|dylib|jar)$/i;
const NOT_TEXT_TO_READ = [
  /(^|\/)(package-lock\.json|Cargo\.lock|bun\.lockb?|yarn\.lock|pnpm-lock\.yaml)$/,
  /transcript-artwork\.json$/, // generated, embeds base64
  /^crates\/gespan-gpui\/assets\/native-editor\.js$/, // minified bundle
];

function trackedFiles(): string[] {
  return execFileSync("git", ["ls-files", "-z"], {
    cwd: root,
    maxBuffer: 64 * 1024 * 1024,
  })
    .toString("utf8")
    .split("\0")
    .filter(Boolean);
}

function read(rel: string): string | null {
  if (BINARY.test(rel)) return null;
  try {
    const buf = readFileSync(join(root, rel));
    return buf.includes(0) ? null : buf.toString("utf8");
  } catch {
    return null;
  }
}

function scan(
  include: (rel: string) => boolean,
  hit: (line: string, rel: string) => boolean,
): string[] {
  const hits: string[] = [];
  for (const rel of trackedFiles()) {
    if (!include(rel)) continue;
    const text = read(rel);
    if (text === null) continue;
    text.split(/\r?\n/).forEach((line, i) => {
      if (hit(line, rel)) hits.push(`${rel}:${i + 1}`);
    });
  }
  return hits;
}

const readable = (rel: string) => !NOT_TEXT_TO_READ.some((re) => re.test(rel));
const own = (rel: string) => rel !== "src/test/legacyNames.test.ts";
const ownCode = (rel: string) => readable(rel) && own(rel) && !rel.startsWith("vendor/");

describe("the first product name", () => {
  const names = new RegExp(`${FIRST_NAME}|${FIRST_SITE}`, "i");

  it("appears nowhere: not in code, docs, comments or vendored notices", () => {
    expect(
      scan(
        (rel) => readable(rel) && own(rel),
        (l, rel) => names.test(withoutCredit(l, rel)),
      ),
    ).toEqual([]);
  });

  it("appears in no file or folder name", () => {
    expect(trackedFiles().filter((p) => names.test(p))).toEqual([]);
  });

  it("is not in the upstream repository URL either", () => {
    const upstream = new RegExp(`hardbeat920/${FIRST_NAME}`, "i");
    expect(scan(ownCode, (l, rel) => upstream.test(withoutCredit(l, rel)))).toEqual([]);
  });

  it("is credited in the README, as a link to the original repository", () => {
    const readme = readFileSync(join(root, CREDITED_IN), "utf8");
    expect(readme.match(CREDIT_LINK)?.length ?? 0).toBeGreaterThan(0);
  });

  it("is allowed only as that credit link, and only in the README", () => {
    const Name = cat("Mo", "no", "Co", "de");
    const credit = `Gespan is a fork of [${Name}](${UPSTREAM_URL}).`;
    expect(names.test(withoutCredit(credit, CREDITED_IN))).toBe(false);
    const stem = new RegExp(FIRST_WORD, "i");
    expect(stem.test(stripTypography(withoutCredit(credit, CREDITED_IN)))).toBe(false);

    for (const [line, file] of [
      [credit, "docs/providers.md"],
      [credit, "src/app/App.tsx"],
      [`Open ${Name} and pick an agent.`, CREDITED_IN],
      [`[${Name}](https://example.com)`, CREDITED_IN],
      [`${credit} ${Name} also runs Codex.`, CREDITED_IN],
      [`See ${UPSTREAM_URL} for details.`, CREDITED_IN],
    ]) {
      expect(names.test(withoutCredit(line, file)), `${file}: ${line}`).toBe(true);
    }
  });
});

describe("the old name for the agent feature", () => {
  const stem = new RegExp(FIRST_WORD, "i");

  it("is gone, apart from typography such as monospace fonts", () => {
    expect(
      scan(ownCode, (l, rel) => stem.test(stripTypography(withoutCredit(l, rel)))),
    ).toEqual([]);
  });

  it("keeps typography words and font names that merely contain the stem", () => {
    for (const ok of [
      "font-family: ui-monospace, 'JetBrains Mono', 'SF Mono', Menlo",
      "var(--font-mono)",
      "a monorepo with a monochrome theme",
      '"Roboto Mono", "IBM Plex Mono", "Liberation Mono"',
      "monopolize the thread",
    ]) {
      expect(stem.test(stripTypography(ok)), ok).toBe(false);
    }
    for (const stray of ["MonoScreen", "const monoList = []", "open the Mono tab"]) {
      expect(stem.test(stripTypography(stray)), stray).toBe(true);
    }
  });
});

describe("the name used before Gespan", () => {
  const word = SECOND_NAME;
  const Word = SECOND_NAME[0].toUpperCase() + SECOND_NAME.slice(1);

  it("is not the product name anywhere in the code, docs or file names", () => {
    expect(
      scan(
        (rel) => ownCode(rel),
        (line, rel) => usesSecondName(line, rel),
      ),
    ).toEqual([]);
    expect(
      trackedFiles().filter(
        (p) => !p.startsWith("vendor/") && usesSecondName(p.replaceAll("/", " ")),
      ),
    ).toEqual([]);
  });

  it("is still allowed to mean the git command and GitHub's merge method", () => {
    for (const ok of [
      `git ${word} --continue`,
      `support.function.git-${word}`,
      `args.push("--${word}")`,
      `${word}Path(file.path, from, to)`,
      `${word}ProjectData(from, to)`,
      `fn ${word}_project(conn: &Connection)`,
      `state.${word}_pending_scroll`,
      `commits ${word}d onto main, then ${word}s again, ${word}`.replace(
        `, ${word}`,
        "",
      ),
    ]) {
      expect(usesSecondName(ok), ok).toBe(false);
    }
  });

  it("keeps the merge-method picker working only in the inbox files", () => {
    const merge = `"squash" => "Squash and merge", "${word}" => "${Word} and merge"`;
    expect(usesSecondName(merge, "crates/gespan-gpui/src/inbox/ui.rs")).toBe(false);
    expect(usesSecondName(merge, "src/features/skills/model/skills.ts")).toBe(true);
    expect(usesSecondName(`source: "${word}"`, "src/features/skills/model/skills.ts")).toBe(true);
  });

  it("catches product uses in every spelling, but never firebase", () => {
    for (const bad of [
      `${word}-core`,
      `${word}_core::Store`,
      `${Word}Link`,
      `useOpen${Word}()`,
      `${word.toUpperCase()}_APP_TOKEN`,
      `com.${word}.desktop`,
      `lib${word}_mobile_link`.replace(`lib${word}`, `${word}`),
      `clientInfo: { name: "${word}" }`,
      `"\\n${word}"`,
      `${Word}.app`,
    ]) {
      expect(usesSecondName(bad), bad).toBe(true);
    }
    for (const ok of [
      "import { initializeApp } from 'firebase/app'",
      "FIREBASE_API_KEY=abc",
      "@firebase/firestore",
      ".firebaserc",
    ]) {
      expect(usesSecondName(ok), ok).toBe(false);
    }
  });
});
