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
- **Tailwind v4** is loaded in `src/styles/motion.css` with a `tw:` prefix,
  without its preflight reset, and inside cascade layers, so it can't restyle
  the site. It supplies the hero's entrance keyframes (`tw:animate-rise`) and
  the drifting glow.
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
