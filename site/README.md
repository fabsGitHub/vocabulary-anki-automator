# Product website

The public page for Polywort (formerly Vocabulary Automator): what the app does, what it
sends where, and the macOS download with install steps. English at `/`, German at `/de/`, plus
a 404 page. It is plain static HTML with one stylesheet and one small script. It loads nothing
from other servers, sets no cookies and has no analytics. Production URL:
<https://polywort.vercel.app> (Vercel project `polywort`).

## Layout

| Path                    | What it is                                                               |
| ----------------------- | ------------------------------------------------------------------------ |
| `build.mjs`             | Renders the pages into `dist/` (Node built-ins only, no npm packages)    |
| `build.test.mjs`        | `node:test` checks of the built output                                   |
| `contrast.test.mjs`     | WCAG contrast of the colour tokens in `src/styles.css`, light and dark   |
| `serve.mjs`             | Local preview server with the headers from `vercel.json`                 |
| `vercel.json`           | Clean URLs and security headers (CSP); the build copies it into `dist/`  |
| `src/page.html`         | The page template, rendered once per language                            |
| `src/404.html`          | The 404 page, in both languages at once                                  |
| `src/i18n/en.json`, `…` | All page copy; both files must have the same keys                        |
| `src/styles.css`        | The stylesheet (light and dark), including the drawing of the app window |
| `src/main.js`           | Optional enhancements: OS hint, checksum copy button, closing the menu   |
| `src/partials/mark.svg` | The header mark: `branding/glyph.svg`, coloured by the stylesheet        |
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
`https://polywort.vercel.app` and must be an origin without a path, because every
link in the page is root-relative. `node site/build.mjs --out <dir> --downloads <dir>` builds
somewhere else. The version comes from `src-tauri/tauri.conf.json`. The build renders
everything before it empties the output folder, so a failed build leaves the previous one in
place.

## Publishing a macOS build

The page describes one disk image: the universal build (Apple Silicon and Intel) of Polywort in
the version of `src-tauri/tauri.conf.json`, ad-hoc signed and not notarized, built from the app
code the copy describes. The copy describes language recognition, “My languages”, field
matching and the macOS 26 copy fix, which `main` gets only when `claude/auto-language` is
merged, and it calls the app Polywort: in Finder, the Dock, the menu bar and the Accessibility
list. Build from app code that has both, that is, whose `productName` in
`src-tauri/tauri.conf.json` is `Polywort`. Code with the old `productName` builds
`Vocabulary Automator.app` in `Vocabulary Automator_<version>_universal.dmg`, and the build
refuses that image. The identifier (`com.ankiautomator.app`) and the data folders keep their
old names, so the paths on the page stay right.

1. Build the disk image from the repository root (the two Rust targets are needed once):

   ```bash
   rustup target add aarch64-apple-darwin x86_64-apple-darwin
   APPLE_SIGNING_IDENTITY=- npm run tauri build -- --target universal-apple-darwin --bundles dmg \
     --config '{"bundle":{"createUpdaterArtifacts":false}}'
   ```

   `APPLE_SIGNING_IDENTITY=-` signs ad hoc (there is no Developer ID yet). The `--config`
   override skips the updater artifacts, whose signing key only CI holds
   ([docs/signing-key-custody.md](../docs/signing-key-custody.md)).

2. Check the app before you publish it:

   ```bash
   APP="src-tauri/target/universal-apple-darwin/release/bundle/macos/Polywort.app"
   codesign -dv "$APP" 2>&1 | grep Signature   # Signature=adhoc
   lipo -archs "$APP"/Contents/MacOS/*         # x86_64 arm64
   ```

3. Copy `src-tauri/target/universal-apple-darwin/release/bundle/dmg/Polywort_<version>_universal.dmg`
   into `site/downloads/`, remove older images from there, and run `npm run site:build`.

The build copies the newest `.dmg` in `site/downloads/` to `dist/downloads/` under a name
without spaces, and writes its size and SHA-256 into the page. It stops with an error unless
that file name is `Polywort_<version>_universal.dmg`, Tauri's name for the universal build of
Polywort in the version of `tauri.conf.json`: a per-architecture image, an image of the app
under its old name or one of another version would be published with a false label.
`--allow-version-mismatch` lets another version through with a warning, but the page still
shows the version from `tauri.conf.json`; nothing lets another app name through. Without a
`.dmg`, the macOS buttons link to GitHub Releases and the page says that the download is not
available yet.

## Deploying

`dist/` is the whole static site, `vercel.json` included, and it is the only folder to deploy.
The Vercel CLI keeps its link to the project in `site/dist/.vercel/`, which git ignores and the
build keeps when it empties `site/dist`:

```bash
npm run site:build
vercel link --cwd site/dist --project polywort   # once, and again if site/dist/.vercel is gone
vercel deploy --cwd site/dist --prod
```

Before you confirm `vercel link` or `vercel deploy`, check that the project the CLI names is
`polywort`. If it offers to set up a new project (for example one called `dist`) or names any
other project, `vocabulary-automator` (the name before the rename) included, cancel, delete
`site/dist/.vercel` and run `vercel link` again with `--project`. The
production URL is <https://polywort.vercel.app>, the build's default `SITE_URL`. Never deploy
from the repository root or `site/`: neither is the site.

## Mark, icons and colours

The artwork comes from `branding/` at the repository root (see its README). The header mark,
`src/partials/mark.svg`, is the path of `branding/glyph.svg` filled with `currentColor`, so the
stylesheet colours it, and `src/assets/favicon.svg` is a copy of `branding/favicon.svg`.
`og/render.mjs` draws `apple-touch-icon.png` from `branding/icon.svg`, cropped to the icon's
body and filled to the corners (iOS rounds them itself), and puts the mark and the headline
with its selected word on the social previews. When the branding changes, update the two SVG
files and run `node site/og/render.mjs`.

The colours are the tokens at the top of `src/styles.css`, taken from the icon: raspberry for
the brand and the primary buttons, violet for the selected word in the hero, text selections
and focus rings, plum ink for headings. `contrast.test.mjs` checks every text and UI colour
against the backgrounds it sits on, in light and in dark mode, and fails below WCAG AA. The
approval-window drawing keeps the app's own colours, its blue button included.

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
