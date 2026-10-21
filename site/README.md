# openstance site

The public landing page for openstance: a small static site built with Vite, React and TypeScript.
It is separate from the local web UI that `openstance serve` embeds (that lives in `ui/`).

The visual language (palette, Georgia display headings, mono kickers, terminal card, cards and chips)
is ported from the v0.1 browser prototype. The copy follows the repository README and `docs/product/*`;
anything not shipped yet is labelled as such (the local web UI, v1.0 playbooks, the cloud tier).

## Develop

```sh
cd site
npm ci
npm run dev        # http://localhost:5173
```

## Check

```sh
npm run lint
npm run typecheck
npm test           # Vitest + Testing Library, no network
npm run build      # writes dist/
npm run preview    # serves dist/ on http://localhost:4173
```

Tests cover the hero, the required sections, the CLI/MCP/WEB tabs (click and arrow keys), honest
status labels, and a scan of `index.html`, `src/` and `public/` for anything the browser would fetch
from another origin (CDN fonts, remote images, `fetch`, `@import`, and so on).

## Constraints

- **No external requests.** System fonts only, inline SVG icons, the favicon is a local file.
  The build adds a strict Content-Security-Policy meta tag (`default-src 'self'`, `connect-src 'none'`,
  no inline scripts or styles). It is build-only because the dev server injects an inline script for
  React Fast Refresh.
- **Outbound links** go only to the GitHub repository (`src/links.ts`) and open with
  `rel="noopener noreferrer"`.
- **Example output** in the terminal card, score card and install snippet comes from the README
  quickstart and `docs/tech-spec/interfaces.md`, run against the fictional dataset in `fixtures/demo/`.
  Do not put real contact data here, and do not add metrics that the docs do not support.
- **Responsive** down to 375 px. The tabs follow the WAI-ARIA tabs pattern (roving tabindex,
  arrow keys, Home and End).
- `base: './'` makes the build path-independent, so `dist/` works from any sub-path.

## Deploy

`npm run build` produces a fully static `dist/` (one HTML file, one JS and one CSS bundle, a favicon).
Upload it to any static host: GitHub Pages, Cloudflare Pages, Netlify or an S3 bucket.

For GitHub Pages from Actions, build with Node 22 or later and publish `site/dist`:

```yaml
- run: npm ci && npm run build
  working-directory: site
- uses: actions/upload-pages-artifact@v3
  with: { path: site/dist }
```

If the host can set response headers, prefer sending the same Content-Security-Policy as a header,
plus `Referrer-Policy: no-referrer` and `X-Content-Type-Options: nosniff`.
