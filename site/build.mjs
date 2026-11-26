import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { guides, renderGuide } from './guides.mjs';

const { values: { url } } = parseArgs({ options: { url: { type: 'string' } } });
const source = fileURLToPath(new URL('.', import.meta.url));
const require = createRequire(import.meta.url);
const font = dirname(require.resolve('@fontsource-variable/plus-jakarta-sans/package.json'));
const shareImage = './assets/dgi-share.webp';

function publicOrigin(value) {
  if (!value) return null;
  const origin = new URL(value);
  if (origin.protocol !== 'https:' || !origin.hostname.includes('.') || origin.port ||
      origin.username || origin.password || origin.pathname !== '/' || origin.search || origin.hash) {
    throw new Error('Use a public HTTPS origin, such as https://dgi.example.com, with no path or query.');
  }
  return origin.href;
}

function withSearchMetadata(html, origin) {
  if (!origin) {
    return html.replace('</head>', '    <meta name="robots" content="noindex, nofollow">\n  </head>');
  }
  const image = new URL(shareImage, origin).href;
  const software = {
    '@context': 'https://schema.org',
    '@type': 'SoftwareApplication',
    name: 'DGI Cache Studio',
    applicationCategory: 'UtilitiesApplication',
    operatingSystem: 'Windows',
    url: origin,
    image,
    description: 'A Windows desktop utility for browsing and previewing locally cached Roblox assets.',
  };
  const metadata = {
    '@context': 'https://schema.org',
    '@graph': [
      { '@type': 'WebSite', name: 'DGI Cache Studio', alternateName: 'DGI', url: origin },
      software,
    ],
  };
  return html.replaceAll(shareImage, image).replace('</head>', [
    `    <link rel="canonical" href="${origin}">`,
    `    <meta property="og:url" content="${origin}">`,
    `    <script type="application/ld+json">${JSON.stringify(metadata)}</script>`,
    '  </head>',
  ].join('\n'));
}

const origin = publicOrigin(url);
const product = resolve(source, 'dist', origin ? 'dgi' : 'preview');
const root = resolve(source, 'dist/root');
const template = await readFile(resolve(source, 'index.html'), 'utf8');
const html = withSearchMetadata(template, origin);
await Promise.all([mkdir(resolve(product, 'assets'), { recursive: true }), mkdir(root, { recursive: true })]);

const sitemap = origin && new URL('sitemap.xml', origin).href;
const robots = 'User-agent: *\nAllow: /\n' +
  (sitemap ? `\nUser-agent: OAI-SearchBot\nAllow: /\n\nSitemap: ${sitemap}\n` : '');
const pages = [
  { path: '', image: 'dgi-preview.webp' },
  ...guides.map(guide => ({ path: `guides/${guide.slug}/`, image: guide.image })),
];
await Promise.all([
  writeFile(resolve(product, 'index.html'), html),
  writeFile(resolve(product, 'robots.txt'), robots),
  copyFile(resolve(source, 'root/index.html'), resolve(root, 'index.html')),
  copyFile(resolve(source, '../src/design-system.css'), resolve(product, 'design-system.css')),
  copyFile(resolve(source, '../src-tauri/icons/128x128.png'), resolve(product, 'favicon.png')),
  copyFile(resolve(font, 'files/plus-jakarta-sans-latin-wght-normal.woff2'), resolve(product, 'assets/plus-jakarta-sans.woff2')),
  copyFile(resolve(font, 'LICENSE'), resolve(product, 'assets/plus-jakarta-sans-LICENSE.txt')),
  ...['styles.css', 'theme.js', 'carousel.js', 'assets/dgi-preview.webp', 'assets/dgi-animation.webp', 'assets/dgi-audio.webp',
    'assets/dgi-changes.webp', 'assets/dgi-share.webp', 'assets/dgi-wordmark.png']
    .map(file => copyFile(resolve(source, file), resolve(product, file))),
  ...guides.map(async guide => {
    const directory = resolve(product, 'guides', guide.slug);
    await mkdir(directory, { recursive: true });
    await writeFile(resolve(directory, 'index.html'), renderGuide(guide, origin));
  }),
  ...(origin ? [writeFile(resolve(product, 'sitemap.xml'),
    '<?xml version="1.0" encoding="UTF-8"?>\n' +
    '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">\n' +
    pages.map(page => `  <url><loc>${new URL(page.path, origin).href}</loc><image:image><image:loc>${new URL('assets/' + page.image, origin).href}</image:loc></image:image></url>`).join('\n') +
    '\n</urlset>\n',
  )] : []),
]);

console.log(`DGI ${origin ? 'production site' : 'preview'}: ${product}`);
console.log(`Blank main site: ${root}`);
