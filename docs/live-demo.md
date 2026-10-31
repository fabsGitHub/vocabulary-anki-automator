# Live demo

A public demo of Learn that anyone can open in a browser: the web build of
Learn, started with a made-up history so that every tab has something to
show. It needs no server and no account; whatever a visitor does stays in
their browser.

## What a visitor sees

- **Ten evenings of practice already done**: 43 words learned (the first 35
  HSK words and eight words "captured in the Automator", four Chinese and
  four German, all with English meanings), a ten-day streak with one streak
  freeze earned, and a few reviews due today.
- **Two captured words still waiting**, captured after yesterday's session,
  so today's queue shows the order of § 7.1 of
  [architecture.md](architecture.md): reviews, then captures, then the next
  HSK words.
- **Two lessons** (Chinese → English and German → English, five words each),
  and exercises built from the learned words.
- **A bar above the header** that says it is a demo, with **Next day** (moves
  the demo's clock one day ahead, so the visitor can watch cards come back
  and the streak go on or be saved by its freeze), **Start over**, and a
  folded paragraph about the project.

Like the web build it has no text-to-speech (the listening exercises are
not offered) and no Anki access.

## How it works

`--mode demo` builds the web build with a different entry point,
`apps/learn/src/demo/main.ts`; nothing of the demo is in the desktop app or
the plain web build (`apps/learn/src/platform/builds.test.ts` checks this).

- **Its own database.** The demo uses the IndexedDB database
  `vocab-learn-demo`, never the web build's `vocab-learn`, so it cannot touch
  anyone's own words even on a shared origin (a GitHub Pages user site serves
  every project from one origin). Where the browser refuses IndexedDB, the
  demo runs in memory and its bar says that a reload starts it over.
- **A played history.** `apps/learn/src/demo/history.ts` lists the captured
  words (`DEMO_WORDS`) and the evening sessions (`DEMO_SESSIONS`). They are
  played through the web build's own commands on a clock that walks through
  the past ten local days, with ratings from a seeded generator, so every row
  is one the app itself writes and every visitor gets the same history, ending
  yesterday evening in their own time zone. The play runs in memory with only
  the start of the word list; `apps/learn/src/demo/demo.ts` then stores its
  rows, the whole curriculum and the demo's own `app_state` keys
  (`demo.prepared_for`, `demo.day_shift`) in one transaction.
- **When it starts over.** The history is prepared on the first visit (about
  a second on a fast connection, most of it downloading the 2 MB curriculum),
  again when it is a week old, and whenever the visitor presses **Start over**.
  Otherwise the visitor finds the demo as they left it.
- **The clock.** **Next day** does not change the data: the app reads one
  clock, shifted by whole days, for the core and for the UI alike, and the
  shift is stored with the demo. After each move or restart the app mounts
  afresh, as on a new start.

To change the history, edit `DEMO_WORDS` and `DEMO_SESSIONS`. Keep the
captured words off the HSK list (an HSK word waits for its turn in the list
instead of being offered as a capture) and the hanzi of the Chinese example
sentences within the words learned by today (the sentence builder shows
learned hanzi only); `apps/learn/src/demo/demo.test.ts` checks both and pins
what a visitor starts with.

## Build and try it locally

```bash
npm run learn:dev:demo
```

```bash
npm run learn:build:demo
```

```bash
npm run learn:preview:demo
```

The first serves the demo from source at <http://localhost:1430> (also
`just learn-demo`). The second writes the static files to
`apps/learn/dist-demo`, and the third serves that folder at
<http://localhost:4173>. To add a "Project page" link to the demo's bar,
build it with `VITE_DEMO_PROJECT_URL` set to the page that presents the
project, for example
`VITE_DEMO_PROJECT_URL=https://example.com/projects/vocabulary npm run learn:build:demo`.

## Put it online

`apps/learn/dist-demo` is a folder of static files with relative paths: an
`index.html` and an `assets/` folder (the app, about 95 KB gzipped, its
stylesheet, and the curriculum JSON, about 550 KB gzipped). It has no routes
and needs no rewrites, redirects or headers, so any static host can serve it
from any folder.

### Cloudflare Pages (the chosen host)

Cloudflare Pages builds the demo from this repository (private repositories
work, on the free plan) and builds it again on every push to the branch it
deploys. Set it up once in the Cloudflare dashboard:

1. **Workers & Pages** > **Create application** > **Pages** > **Connect to
   Git**, then **GitHub**. Give Cloudflare's GitHub app access to this
   repository only (**Install & Authorize**), pick it and choose **Begin
   setup**.
2. In **Set up builds and deployments**:
   - **Project name**: the start of the address, for example
     `vocabulary-learn` for `https://vocabulary-learn.pages.dev/`.
   - **Production branch**: the branch with the demo, `wip/live-demo` until
     it is merged, then `main`.
   - **Framework preset**: None.
   - **Build command**: `npm run learn:build:demo`
   - **Build output directory**: `apps/learn/dist-demo`
   - **Root directory**: leave it empty (the repository root).
   - **Environment variables**: `NODE_VERSION` = `22`, and optionally
     `VITE_DEMO_PROJECT_URL` = the page that presents the project.
3. **Save and Deploy**. Pages installs the npm dependencies itself; the
   build takes about a minute.

Then, under the project's **Settings** > **Builds & deployments**:

- Pages also builds every other branch you push, as a preview with an
  address of its own. Set preview deployments to **None** if you do not want
  that; branches without the demo (older than it) fail to build.
- Once the demo is on `main`, make `main` the production branch.

**Custom domains** in the project adds an address of your own, such as
`demo.your-site.com`.

### Other hosts

1. **Your own website**, if it is static files: copy the contents of
   `apps/learn/dist-demo` into a folder of the site, for example
   `/demos/vocabulary/`. The demo then shares your site's address, and an
   embedded demo keeps its data like any page of the site.
2. **Netlify**: the same settings as Cloudflare Pages (build command,
   publish directory, `NODE_VERSION`).
3. **GitHub Pages**, with `.github/workflows/demo.yml`: it needs the
   repository to be public, or a GitHub plan that includes Pages for private
   repositories. Choose **GitHub Actions** as the source under Settings >
   Pages, then run **Actions > Live demo > Run workflow** whenever the demo
   should be updated. The demo is then at
   `https://<owner>.github.io/vocabulary-anki-automator/`. The repository
   variable `DEMO_PROJECT_URL` adds the project link.

If the host sets a Content Security Policy, the demo needs what the web
build needs ([architecture.md](architecture.md) § 10): scripts, styles and
the curriculum JSON all come from its own folder.

## Embed it in a web page

```html
<iframe
  src="https://your-demo-address/"
  title="Vocabulary Learn – live demo"
  width="100%"
  height="680"
  style="border: 1px solid #d1d1d6; border-radius: 12px"
  loading="lazy"
></iframe>
```

- The layout fits frames from about 360 pixels wide, and the keyboard
  shortcuts (Space, 1 to 4) work once the frame has focus.
- A demo embedded from another site keeps its data separately for each site
  that embeds it, as browsers do with all storage in embedded pages, and some
  browsers clear it sooner (Safari after seven days without a visit). Where a
  browser refuses storage in embedded pages altogether, the demo runs in
  memory. Hosting the demo on your own site avoids all of this.
- If the host sends a `frame-ancestors` policy or `X-Frame-Options`, it must
  allow your site. The hosts above send neither by default.
