# Product website

The public page for Vocabulary Automator: what the app does, what it sends where, and the
macOS download with install steps. English at `/`, German at `/de/`, plus a 404 page. It is
plain static HTML with one stylesheet and one small script. It loads nothing from other
servers, sets no cookies and has no analytics. Expected production URL:
<https://vocabulary-automator.vercel.app>.

## Layout

| Path                    | What it is                                                               |
| ----------------------- | ------------------------------------------------------------------------ |
| `build.mjs`             | Renders the pages into `dist/` (Node built-ins only, no npm packages)    |
| `build.test.mjs`        | `node:test` checks of the built output                                   |
| `serve.mjs`             | Local preview server with the headers from `vercel.json`                 |
| `vercel.json`           | Clean URLs and security headers (CSP); the build copies it into `dist/`  |
| `src/page.html`         | The page template, rendered once per language                            |
| `src/404.html`          | The 404 page, in both languages at once                                  |
| `src/i18n/en.json`, `…` | All page copy; both files must have the same keys                        |
| `src/styles.css`        | The stylesheet (light and dark), including the drawing of the app window |
| `src/main.js`           | Optional enhancements: OS hint, checksum copy button, closing the menu   |
| `src/assets/`           | Favicon, touch icon and the social preview images                        |
| `og/render.mjs`         | Redraws the social preview images and the touch icon                     |
| `downloads/`            | Input folder (git-ignored) for the `.dmg` to publish                     |

## Build and preview

```bash
npm run site:build      # renders site/dist
npm run site:preview    # serves site/dist at http://localhost:4173 with the vercel.json headers
npm run site:test       # builds into temporary folders and checks the output
```

`SITE_URL` sets the absolute URLs (canonical, hreflang, Open Graph, sitemap); it defaults to
`https://vocabulary-automator.vercel.app`. `node site/build.mjs --out <dir> --downloads <dir>`
builds somewhere else. The version comes from `src-tauri/tauri.conf.json`.

## Publishing a macOS build

Put the universal disk image into `site/downloads/` and build. The build copies the newest
`.dmg` there to `dist/downloads/` under a name without spaces, and writes its size and
SHA-256 into the page. Without a `.dmg`, the macOS buttons link to GitHub Releases and the
page says that the download is not available yet.

## Deploying

`dist/` is the whole static site, `vercel.json` included. From a directory linked to the
Vercel project `vocabulary-automator` (`vercel link` writes a `.vercel/` folder, which git
ignores and the build keeps when it empties `site/dist`):

```bash
vercel deploy site/dist --prod
```

## Changing the copy

Every sentence on the page has to be true of the code in this repository. UI names are
quoted exactly as the app shows them (`src/i18n/en.ts`, `src/i18n/de.ts`), the German copy
addresses the reader as „Sie“, and the page never calls the project open source or names a
license for it. The app window on the page is a drawing with the app's real labels; it is
not a screenshot. After changing the copy or the mark, redraw the social preview images with
`node site/og/render.mjs` (needs `rsvg-convert` from librsvg, works offline).

Template syntax (`build.mjs` has the details): `{{group.key}}` inserts copy, HTML-escaped
unless the key ends in `_html`; `{{build.name}}` inserts a value the build computes;
`{{#dmg}}…{{/dmg}}` and `{{^dmg}}…{{/dmg}}` keep a block with or without a disk image;
`{{icon:name}}` and `{{>mark}}` include artwork. Copy can use `{version}`, `{dmgFile}`,
`{dmgSize}`, `{ankiUrl}` and `{mymemoryUrl}`.

The Content-Security-Policy allows no inline scripts, inline styles or event handlers, and
the tests reject them, along with any external URL that is not on their short allowlist.
