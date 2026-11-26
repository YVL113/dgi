# DGI Cache Studio website

The landing page lives at [dgi.nok02.cc](https://dgi.nok02.cc/). It shares Ghost Acrylic tokens with the desktop app and uses the system color scheme, with light as the default.

## Preview locally

Run these commands from the repository root.

```sh
npm ci --prefix site
node site/build.mjs
python -m http.server 4175 --bind 127.0.0.1 --directory site/dist
```

Open [the local preview](http://127.0.0.1:4175/preview/). The preview build includes a `noindex` directive. The blank root template is available separately under `/root/`.

## Deploy to Cloudflare

```sh
npm ci --prefix site
node site/build.mjs --url https://dgi.nok02.cc
npx --yes wrangler@4.168.0 deploy --config site/wrangler.jsonc
```

The production build sets the canonical URL, social metadata, and sitemap for `dgi.nok02.cc`. Wrangler publishes `site/dist/dgi` to the Worker attached to that hostname. Sign in to the Cloudflare account that manages `nok02.cc` before deploying.

The Windows Builds links open the successful desktop builds in GitHub Actions. The `DGI-windows-x64` installer artifact requires repository access.
