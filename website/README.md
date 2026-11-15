# gespan.dev

The Gespan website: React 19 on Vite 8. Vite bundles with Rolldown and
transforms with Oxc, React Compiler runs through Oxc (`oxc-transform-react`),
and oxlint lints.

```bash
cd website
bun install
bun run dev      # http://localhost:5173
bun run build    # dist/, one index.html per page plus 404.html
bun run test     # Vitest
bun run lint     # oxlint
```

## Where the content comes from

- **Downloads and the version** come from the release block in the root
  `README.md`, which `set-version` rewrites on every release. The build fails
  when a download is missing or still points at an older version.
- **The Changelog page** is the newest releases in the root `CHANGELOG.md`,
  read at build time.
- **Product shots** (`src/shots/`) are the app's glass screens from the Gespan
  design canvas, drawn in a shadow root and scaled to their frame, so they are
  never cropped. The demo project in them is `acme-web`.
- **Styles** follow the Gespan design system's website glass extension:
  `src/styles/site.css` holds the shared classes, and each page keeps its own
  CSS beside it. Layouts respond to the page's width with container queries.

## Motion

- **Motion** (`motion/react`, loaded through `LazyMotion`) drives the
  scroll-linked product shots in `src/components/ScrollShot.tsx`: the hero
  screenshot starts tilted back and straightens as you scroll, and the other
  shots rise into place. The poses are plain functions (`heroPose`,
  `risePose`) with tests.
- **Tailwind v4** (with `tw-animate-css`) is loaded in `src/styles/motion.css`
  without its preflight reset and inside cascade layers, so it can't restyle
  the site. A test checks that no class the site defines is also a Tailwind
  utility. It supplies the hero's entrance keyframes (`animate-rise`) and
  the drifting glow.
- **shadcn and Magic UI.** `components.json` sets up the shadcn CLI, and
  `src/components/ui/` holds Magic UI components added with it (marquee,
  animated beam, border beam, shiny text, word rotate, magic card, number
  ticker, animated list, particles, scroll progress). They're ours to edit;
  edits are marked "Gespan:". `src/components/fx.tsx` composes them into the
  site's pieces: the flow diagram, agent marquee, task feed, stat band,
  spotlight cards, rotating hero word and beams. Add more with
  `bunx shadcn add https://magicui.design/r/<name>.json`.
- **Scroll reveals:** add `data-reveal` to an element, or `data-stagger` to a
  grid whose children should follow one another. `useReveal` marks each one as
  it enters the viewport.
- Visitors who prefer reduced motion get every element at rest, and content
  is never hidden while the reveal watcher isn't running.

## Pages

`src/routes.json` lists the pages. The router is a small history router
(`src/router.tsx`), and `scripts/copy-routes.mjs` writes each page's
`index.html` after the build, so any static host can serve `/features`
directly.
