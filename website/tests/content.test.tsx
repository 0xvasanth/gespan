import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { render } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import { renderInline } from "../src/components/Inline";
import { GUIDES, REPO } from "../src/docs";
import { DOC_GUIDES, filterGuides } from "../src/pages/DocsPage";
import { highlights } from "../src/release";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = join(site, "..");

describe("docs links", () => {
  test("every guide the site links to exists in the repository", () => {
    for (const href of Object.values(GUIDES)) {
      const path = href.replace(`${REPO}/blob/main/`, "").replace(/#.*$/, "");
      expect(existsSync(join(repoRoot, path)), path).toBe(true);
    }
  });

  test("search matches every word in any order and case", () => {
    expect(filterGuides("").length).toBe(DOC_GUIDES.length);
    expect(filterGuides("JIRA").map((g) => g.title)).toEqual(["Jira inbox"]);
    expect(filterGuides("ssh machine").map((g) => g.title)).toEqual(["Remote sessions"]);
    expect(filterGuides("deb appimage").map((g) => g.title)).toEqual(["Linux installation"]);
    expect(filterGuides("jira ssh")).toEqual([]);
  });
});

describe("changelog text", () => {
  const html = (text: string) => render(<p>{renderInline(text)}</p>).container.innerHTML;

  test("renders bold, code and web links", () => {
    expect(html("Run `bun run dev` for **speed**, see [docs](https://gespan.dev/docs).")).toBe(
      '<p>Run <code class="w-code-inline">bun run dev</code> for <b>speed</b>, see <a class="w-link" href="https://gespan.dev/docs">docs</a>.</p>',
    );
  });

  test("never injects HTML and drops non-web link targets", () => {
    expect(html("<img src=x onerror=alert(1)> and [x](javascript:void0)")).toBe("<p>&lt;img src=x onerror=alert(1)&gt; and x</p>");
  });

  test("a release headline joins its first three titles", () => {
    const entry = { version: "1.0.0", date: null, sections: [{ kind: "Added", entries: ["A", "B", "C", "D"].map((t) => ({ title: t, body: "" })) }] };
    expect(highlights(entry)).toBe("A, B and C");
    expect(highlights({ ...entry, sections: [{ kind: "Added", entries: [{ title: "Only", body: "" }] }] })).toBe("Only");
  });
});

describe("product name", () => {
  // The product is Gespan. Its old names must not reach the public site.
  const OLD = [/\brebase\b/i, /monocode/i, /\bMono\b/];
  const files = (dir: string): string[] =>
    readdirSync(dir).flatMap((name) => {
      const path = join(dir, name);
      return statSync(path).isDirectory() ? files(path) : [path];
    });

  test("no source, page or shot uses an old name", () => {
    const sources = [...files(join(site, "src")), ...files(join(site, "public")), join(site, "index.html")];
    for (const file of sources) {
      // Quoted font names ("Liberation Mono") are fonts, not the product.
      const text = readFileSync(file, "utf8").replace(/"[^"\n]*\bMono\b[^"\n]*"/g, "");
      for (const pattern of OLD) expect(pattern.test(text), `${file} matches ${pattern}`).toBe(false);
    }
  });
});
