# Description Converters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pure functions in `gespan-core` that convert issue descriptions between Markdown and Jira ADF, and between Markdown and Plane HTML, keeping headings, lists, code, links and emphasis, and keeping anything they can't represent instead of dropping it.

**Architecture:** One small document model (`format::doc`) sits in the middle. Each format has a parser into it and a renderer out of it (`markdown.rs`, `adf.rs`, `html.rs`). A single `normalize` pass defines the canonical form, so the round-trip property is `parse_F(render_F(d)) == d` for every normalized document `d` that format `F` can represent, checked with `proptest`, plus a fixture corpus of realistic descriptions. Content the model can't hold becomes `Preserved { source, raw }`: the original ADF node JSON or HTML element source, carried through Markdown in a form that renders back byte-for-byte.

**Tech Stack:** Rust 2021 in `crates/gespan-core`; `pulldown-cmark` 0.13 (already in the native lock file through `gespan-gpui`) for Markdown parsing; `serde_json` for ADF; a small hand-written HTML tokenizer (no new HTML crate); `proptest` 1 as a dev dependency.

**Spec:** GitHub issue #101 (approved design). Background: epic #49, "Description formats" under Part 2 and "Description round-trips" under testing. Working rules: #60.

## Global Constraints

- Code lives under `crates/gespan-core/src/provider_boards/format/**` (new). `provider_boards.rs` (the crate's `foo.rs` + `foo/` layout) only declares `pub mod format;`, and `lib.rs` gains one `pub mod provider_boards;` line (#97 is creating the same parent module in parallel, so both stay one line).
- Pure functions only: no I/O, no network, no global state.
- Round trips keep headings, lists, code, links and emphasis.
- Blocks we can't represent are kept read-only, never dropped.
- Done when: round-trip property tests pass on a corpus of real descriptions.
- `Cargo.toml` and `Cargo.lock` change only through cargo. Gates: `bun run check` and `bun run native:check`.
- Gespan names only. No tool or AI attribution in commits or the PR.

## Public API

In `crates/gespan-core/src/provider_boards/format/mod.rs`:

```rust
pub fn markdown_to_adf(markdown: &str) -> serde_json::Value;
pub fn adf_to_markdown(adf: &serde_json::Value) -> String;
pub fn markdown_to_plane_html(markdown: &str) -> String;
pub fn plane_html_to_markdown(html: &str) -> String;
```

Everything else is `pub(crate)`. Linear is Markdown already and needs no converter.

## Document model (`format/doc.rs`)

```rust
pub(crate) struct Doc { pub blocks: Vec<Block> }
pub(crate) enum Block {
    Paragraph(Vec<Inline>),
    Heading { level: u8, content: Vec<Inline> },          // 1..=6
    BulletList(Vec<Vec<Block>>),                          // items
    OrderedList { start: u64, items: Vec<Vec<Block>> },
    Code { language: Option<String>, code: String },
    Quote(Vec<Block>),
    Rule,
    Html(String),                                         // a raw HTML block written in Markdown
    Preserved(Preserved),
}
pub(crate) enum Inline {
    Text { text: String, marks: Marks },
    HardBreak,
    Image { src: String, alt: String, link: Option<Link> },
    Html(String),                                         // raw inline HTML written in Markdown
    Preserved(Preserved),
}
pub(crate) struct Marks { strong: bool, em: bool, strike: bool, code: bool, link: Option<Link> }
pub(crate) struct Link { href: String, title: Option<String> }
pub(crate) enum Source { Adf, Html }
pub(crate) struct Preserved { source: Source, raw: String }   // raw = compact node JSON, or the exact HTML source slice
impl Preserved { fn fallback_text(&self) -> String }           // derived, never stored
pub(crate) fn normalize(doc: Doc) -> Doc;
```

All derive `Debug, Clone, PartialEq, Eq`.

### Canonical form (`normalize`), applied by every parser

1. In text (not in code blocks): every ASCII whitespace character becomes a space and runs of spaces collapse to one. `\0` becomes U+FFFD.
2. `code` drops `strong`, `em` and `strike` (ADF forbids code combined with them).
3. Adjacent `Text` with equal marks merge; empty `Text` is removed.
4. A `strong`, `em` or `strike` span never starts or ends with a space: the space moves outside that mark (repeat to a fixed point).
5. Plain text (no `code`, no `link`) is trimmed at the start and end of a block's inline content and on both sides of a `HardBreak`. Leading and trailing `HardBreak`s are removed. A `HardBreak` inside a heading becomes a space.
6. Empty paragraphs, headings, list items, lists and quotes are removed.
7. Code blocks: `\r\n` and `\r` become `\n`, trailing newlines are removed; language is the first whitespace-separated word of the info string, empty means `None`.
8. A code block whose language is `gespan-adf` or `gespan-html` becomes `Preserved` with that source and the code as `raw` (this is how preserved blocks travel through Markdown, and through the other format).
9. `Html` blocks lose trailing newlines; ordered-list `start` is capped at 999 999 999.

## Design calls (mine, not in the issue)

- **Markdown dialect:** CommonMark plus `~~strike~~` (`pulldown-cmark` with only `ENABLE_STRIKETHROUGH`). Tables, task lists and footnotes are not parsed; they stay as their literal text. Soft line breaks become spaces.
- **Markdown output:** `#` headings, `-` bullets (alternating with `*` when two bullet lists are adjacent, and `.`/`)` for adjacent ordered lists, so they don't merge), fenced code with a fence longer than any backtick run in the code, `> ` quotes, `---` rules, `\` + newline for hard breaks, `**strong**`, `_em_`, `~~strike~~`, `` `code` ``, `[text](href "title")`, `<href>` autolinks when the text equals an absolute URL.
- **Emphasis that can't be written with delimiters** (for example `**"a"**b`, or intraword `_`) falls back to `<strong>`, `<em>`, `<del>` inline tags, which the Markdown parser reads back as marks. A delimiter is used only when a conservative check of the CommonMark flanking rules, using ASCII whitespace and ASCII punctuation only, says it will parse.
- **Escaping:** `\` `` ` `` `*` `_` `~` `[` `]` `<` `>` always; `&` before a letter or `#`; `!` at the end of a text piece; at the start of a line `#`, `-`, `+`, `=` and the `.`/`)` after leading digits; `#` at the end of a heading.
- **Preserved content in Markdown is read-only.** A block becomes a fenced code block with info `gespan-adf` (compact JSON) or `gespan-html` (the exact HTML). An inline node becomes `<span data-gespan-adf="…">fallback</span>` (or `data-gespan-html`), with the raw value HTML-attribute escaped (`&amp; &quot; &lt; &gt; &#10;`). The parser takes the value from the attribute and ignores the text inside the span, so edits to the fallback text can't corrupt the original. Showing these as locked in the UI is for #104/#105.
- **Preserved content in the other format:** an ADF block in HTML is `<pre><code class="language-gespan-adf">`, an ADF inline in HTML is the same `<span data-gespan-adf>`; an HTML block in ADF is a `codeBlock` with language `gespan-html`, and an HTML inline in ADF is a text node holding the raw HTML. Nothing is lost moving between trackers.
- **ADF scope:** `doc`, `paragraph`, `heading`, `bulletList`, `orderedList` (`attrs.order`), `listItem`, `codeBlock` (`attrs.language`), `blockquote`, `rule`, `text`, `hardBreak`; marks `strong`, `em`, `strike`, `code`, `link` (`href`, `title`). `localId` attributes are ignored (they're ids, not content). Any other node, attribute or block mark (`table`, `panel`, `mediaSingle`, `expand`, `taskList`, `mention`, `emoji`, `status`, `date`, `inlineCard`, alignment) makes that node `Preserved`. A text node with any other mark (`underline`, `textColor`, `subsup`, `annotation`) is preserved whole.
- **ADF nesting rules on output:** a list item whose first block isn't a paragraph or code block gets an empty paragraph first (the parser drops it again). In list items and quotes, which ADF restricts, a heading is written as a bold paragraph, a rule as the text `---`, a nested quote's blocks are inlined, and an HTML block as a `gespan-html` code block. Images become link text (`alt`, or the URL) pointing at the image. These are one-way formatting losses on Markdown → Jira only; the text is kept.
- **Plane HTML scope:** `p`, `h1`–`h6`, `ul`, `ol[start]`, `li`, `pre>code[class=language-*]`, `blockquote`, `hr`, `br`, `strong`/`b`, `em`/`i`, `s`/`del`/`strike`, `code`, `a[href,title]`, `img[src,alt]`. Allowed everywhere and ignored: `class`, `data-tight`, `target`, `rel`. Any other attribute (for example `style`, `data-type="taskList"`) or element (`table`, `image-component`, `mention-component`, `u`, `span`, `div`) makes that element `Preserved` with its exact source. Known inline tags (`u`, `span`, `mention-component`, `sub`, `sup`, `mark`, `kbd`, …) are preserved inline, others as blocks. Comments are dropped; `script`/`style` are preserved whole.
- **HTML output** is plain semantic HTML without Plane's editor classes (`<p>`, `<ul><li><p>…</p></li></ul>`, `<pre><code class="language-x">`), which Plane accepts.
- **Image titles** in Markdown are dropped; images have no other way to carry them.
- **Corpus:** realistic descriptions written by hand from the published ADF schema and Plane's editor output, since live captures belong to #109. Swapping in real captures later only means adding files.

## Review Focus

1. A Jira description with a table or panel between two paragraphs: after ADF → Markdown → ADF the table node is byte-identical and in the same position. Corpus test in Task 5.
2. Text full of Markdown punctuation (`*`, `_`, `#123`, `1. `, `<tag>`, `[x]`, `\`) that arrives as plain text from Jira or Plane: it must come back as the same plain text, not formatting. Unit tests and the generator alphabet in Task 2.
3. A user editing the fallback text inside a preserved span, or deleting the closing `</span>`: the original node still comes back and no text is lost. Test in Task 2.
4. Plane's empty description `<p></p>` and Jira's empty `doc`/`null`: give `""`, and `""` gives an empty doc / empty HTML, no panic. Tests in Tasks 3 and 4.
5. Malformed HTML (unclosed tags, stray close tags, `<` in text, unknown entities) and malformed ADF (wrong `type`, missing `content`, heading level 9): no panic, nothing dropped (preserved). Tests in Tasks 3 and 4.

---

### Task 1: Module scaffold and document model

**Files:**
- Create: `crates/gespan-core/src/provider_boards.rs`, `crates/gespan-core/src/provider_boards/format/mod.rs`, `crates/gespan-core/src/provider_boards/format/doc.rs`
- Modify: `crates/gespan-core/src/lib.rs` (one `pub mod provider_boards;` line)

**Interfaces:**
- Produces: the model and `normalize` above; helpers `Marks::plain()`, `Inline::text(&str, Marks)`.

- [ ] **Step 1: Failing tests in `doc.rs`** for each canonical-form rule: `whitespace_collapses_in_text_but_not_code_blocks`, `code_drops_other_formatting_marks`, `adjacent_runs_with_equal_marks_merge`, `emphasis_edges_move_spaces_outside` (`[strong " a "]` → `[" ", strong "a", " "]` then trimmed at block edge), `trims_plain_text_at_block_edges_and_around_hard_breaks` (a link run `" a"` keeps its space), `drops_empty_blocks_recursively`, `heading_hard_break_becomes_space`, `gespan_code_fence_becomes_preserved`, `normalize_is_idempotent`.
- [ ] **Step 2:** `cargo test -p gespan-core --manifest-path crates/gespan-gpui/Cargo.toml provider_boards` fails to compile / fails.
- [ ] **Step 3:** Implement the model and `normalize`.
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5:** Commit "Add the description document model".

### Task 2: Markdown parser and renderer

**Files:**
- Create: `crates/gespan-core/src/provider_boards/format/markdown.rs`, `crates/gespan-core/src/provider_boards/format/strategies.rs` (`#[cfg(test)]` proptest generators shared by Tasks 2–4)
- Modify: `crates/gespan-core/Cargo.toml` (`pulldown-cmark = { version = "0.13", default-features = false }`, dev `proptest = "1"`), lock file via cargo

**Interfaces:**
- Produces: `pub(crate) fn parse(markdown: &str) -> Doc` and `pub(crate) fn render(doc: &Doc) -> String` in `markdown.rs`; in `strategies.rs`, `pub(crate) fn doc(profile: Profile) -> impl Strategy<Value = Doc>` with `Profile::{Markdown, Adf, Html}` (Adf: no images, no `Html`, no HTML-preserved, ADF nesting limits; Html: no `Html`, no ADF-preserved), and fixed lists of realistic preserved snippets for each source.

- [ ] **Step 1: Failing tests:**
  - `markdown_round_trips_any_document` (proptest, 512 cases): `parse(&render(&d)) == d` for `d = normalize(generated)`; text alphabet includes `a-z`, space, and `*_`[]<>#-+=.!~\&|()1` and `é`.
  - `markdown_render_is_stable`: `render(&parse(&render(&d))) == render(&d)`.
  - Unit: `punctuation_stays_plain_text` (`"#123 *not* 1. [x] <b> \\"` round-trips as one plain text run), `emphasis_falls_back_to_tags_when_flanking_fails` (`**"a"**b` renders with `<strong>` and parses back), `adjacent_lists_stay_separate`, `code_span_with_backticks_and_edge_spaces`, `fence_longer_than_code_backticks`, `tight_and_loose_lists_parse_alike`, `soft_breaks_become_spaces`, `autolink_when_text_is_the_url`, `preserved_inline_survives_edited_fallback` (fallback text changed → same `raw`), `preserved_inline_survives_missing_close_tag` (no panic, `raw` kept, following text kept), `preserved_block_is_a_gespan_fence`.
- [ ] **Step 2:** Run; tests fail.
- [ ] **Step 3:** Implement `parse` (event walk with a block stack; tight-list inline content goes into an implicit paragraph; `<strong>`/`<em>`/`<del>`/`<s>` inline tags toggle marks; `<span data-gespan-…>` becomes `Preserved` and skips to `</span>`; any other inline HTML is `Inline::Html`) and `render` (inline runs rendered with a mark stack: close-and-reopen keeps nesting valid, marks are closed around hard breaks, then each delimiter is checked against its neighbours and swapped for its tag if it wouldn't parse). Generators avoid `Inline::Html` as the first inline of a line (a lone tag on a line is an HTML block in CommonMark).
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5:** Commit "Parse and render Markdown descriptions".

### Task 3: Jira ADF

**Files:**
- Create: `crates/gespan-core/src/provider_boards/format/adf.rs`
- Modify: `format/mod.rs` (`markdown_to_adf`, `adf_to_markdown`)

**Interfaces:**
- Consumes: `markdown::{parse, render}`, `strategies::doc(Profile::Adf)`.
- Produces: `pub(crate) fn parse(adf: &Value) -> Doc`, `pub(crate) fn render(doc: &Doc) -> Value` (always `{"type":"doc","version":1,"content":[…]}`; mark order `strong, em, strike, code, link`).

- [ ] **Step 1: Failing tests:** `adf_round_trips_any_document` (proptest: `parse(&render(&d)) == d`), `adf_through_markdown_round_trips` (proptest: `parse(&render(&markdown::parse(&markdown::render(&d)))) == d`), `unknown_block_node_is_preserved_verbatim` (panel), `unknown_inline_node_is_preserved_with_fallback` (mention → Markdown shows `@Alice`, back to the same JSON), `text_with_unknown_mark_is_preserved_whole` (underline), `block_marks_preserve_the_block` (alignment), `local_ids_are_ignored`, `list_item_starting_with_a_list_gets_an_empty_paragraph`, `restricted_contexts_degrade_without_losing_text` (heading in a list item → bold paragraph), `image_becomes_link_text`, `null_and_empty_docs_give_empty_markdown`, `malformed_nodes_are_preserved_not_dropped` (heading level 9, missing `content`, non-object entries).
- [ ] **Step 2:** Run; tests fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5:** Commit "Convert Jira ADF descriptions to and from Markdown".

### Task 4: Plane HTML

**Files:**
- Create: `crates/gespan-core/src/provider_boards/format/html.rs`
- Modify: `format/mod.rs` (`markdown_to_plane_html`, `plane_html_to_markdown`)

**Interfaces:**
- Consumes: `markdown::{parse, render}`, `strategies::doc(Profile::Html)`.
- Produces: `pub(crate) fn parse(html: &str) -> Doc`, `pub(crate) fn render(doc: &Doc) -> String`.

- [ ] **Step 1: Failing tests:** `html_round_trips_any_document` and `html_through_markdown_round_trips` (proptest), `plane_editor_classes_are_ignored`, `task_list_and_table_are_preserved_verbatim`, `mention_component_is_preserved_inline`, `styled_paragraph_is_preserved`, `entities_decode_and_reencode` (`&amp; &lt; &nbsp; &#39; &#x1F600; &bogus;`), `whitespace_between_blocks_is_ignored`, `empty_plane_description_is_empty`, `malformed_html_does_not_panic_or_drop_text` (unclosed `<strong>`, stray `</p>`, `a < b`, `<p>` inside `<p>`).
- [ ] **Step 2:** Run; tests fail.
- [ ] **Step 3:** Implement a tokenizer and lenient tree builder that records each element's source byte range, then the tree → `Doc` mapping with the attribute allow-list; the renderer escapes `& < >` in text and also `"` in attributes.
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5:** Commit "Convert Plane HTML descriptions to and from Markdown".

### Task 5: Corpus of realistic descriptions

**Files:**
- Create: `crates/gespan-core/src/provider_boards/format/fixtures/jira/*.json`, `fixtures/plane/*.html`, `fixtures/markdown/*.md`, `crates/gespan-core/src/provider_boards/format/corpus_tests.rs`

- [ ] **Step 1: Failing tests** over every fixture (files listed in the test with `include_str!`):
  - Jira: `markdown_to_adf(&adf_to_markdown(&a)) == a` exactly (fixtures are written in canonical form), and the Markdown is stable on a second pass. Fixtures: a bug report (headings, numbered steps, code block, links, emphasis, hard breaks, a mention, an emoji, a status lozenge, a table and a panel), a feature request with nested lists and a quote, a description with an image (`mediaSingle`) and an inline card, and an empty doc.
  - Plane: `html::parse(&markdown_to_plane_html(&plane_html_to_markdown(h))) == html::parse(h)`, every preserved element's source appears unchanged in the output, and the Markdown is stable. Fixtures: TipTap output with editor classes, a task list, a table, an image component, a mention, code with a language, a quote and a rule; and `<p></p>`.
  - Markdown: through ADF and through HTML gives the same `Doc`, and `render(parse(md))` is stable. Fixtures: an agent-written PR-style description and a short bug note.
  - Spot checks on the Jira bug report's Markdown: contains `## Steps to reproduce`, `1. `, ```` ```rust ````, `[logs](https://`.
- [ ] **Step 2:** Run; fail until fixtures and harness exist; fix any converter bug the corpus finds (with a unit test for it).
- [ ] **Step 3:** Commit "Add a round-trip corpus for description converters".

### Task 6: Verify

- [ ] `bun run check` and `bun run native:check` pass; quote the result lines in the PR.
