export const guides = [
  {
    slug: 'roblox-cache-location',
    title: 'Where is the Roblox cache on Windows?',
    link: 'Find the Roblox cache on Windows',
    description: 'Find the Windows folders DGI checks for the Roblox cache, scan local assets, and see why some files are missing from the Library.',
    image: 'dgi-preview.webp',
    alt: 'DGI Cache Studio Library showing locally scanned sample assets and a model preview',
    intro: 'DGI checks several locations because Roblox cache files can live in different folders. The main locations are under your Windows user profile and temporary directory.',
    body: [
      '<h2>Open the cache folder</h2>',
      '<p>Press <kbd>Win</kbd> + <kbd>R</kbd>, paste one of these paths, and press <kbd>Enter</kbd>. Windows expands the variable to the folder for your account.</p>',
      '<ul><li><code>%LOCALAPPDATA%\\Roblox\\rbx-storage</code></li><li><code>%LOCALAPPDATA%\\Temp\\Roblox\\http</code></li><li><code>%LOCALAPPDATA%\\Temp\\Roblox\\http-wob</code></li><li><code>%TEMP%\\Roblox\\http</code> or <code>%TEMP%\\Roblox\\http-wob</code> when your temporary folder is elsewhere.</li></ul>',
      '<p>Some of these folders may be absent. Cached files can have generated names, and some supported assets are stored inside a cache database.</p>',
      '<h2>Scan the cache in DGI</h2>',
      '<ol><li>Open DGI Cache Studio and let <strong>Library</strong> locate the cache.</li><li>Use <strong>Scan cache</strong> for a full scan with progress. <strong>Choose folder</strong> lets you inspect a local export.</li><li>Use <strong>Filters</strong> to narrow the library to images, textures, audio, animations, meshes, or other asset types. Select an item to open its preview.</li></ol>',
      '<h2>Why an asset can be missing</h2>',
      '<p>Roblox can evict cached entries, and some assets never reach disk. DGI shows supported entries available during its scan. If an asset is missing, load it in Roblox and scan again. A missing asset ID or display name usually means the saved data lacked that metadata.</p>',
    ].join('\n'),
  },
  {
    slug: 'preview-roblox-animations',
    title: 'How to preview Roblox R6 and R15 animations',
    link: 'Preview R6 and R15 animations',
    description: 'Find a cached Roblox KeyframeSequence and preview it on an R6 or R15 rig in DGI Cache Studio, with timeline, speed, and loop controls.',
    image: 'dgi-animation.webp',
    alt: 'DGI animation inspector showing an imported R6 rig paused on the timeline',
    intro: 'The animation inspector plays supported Roblox KeyframeSequences on an R6 or R15 preview rig. You can open a cached clip or scan a folder of exported assets.',
    body: [
      '<h2>Find and play a clip</h2>',
      '<ol><li>Open <strong>Library</strong> and scan the Roblox cache or choose your exports folder.</li><li>Open <strong>Filters</strong>, select <strong>Animations</strong>, and choose a clip. Search can match its name or a known asset ID.</li><li>Press <strong>Play</strong> or drag the timeline to inspect an individual pose. Adjust playback speed or turn looping on in the inspector.</li></ol>',
      '<h2>How DGI picks the rig</h2>',
      '<p>DGI identifies R6 or R15 from the sequence’s pose names. The screenshot shows an imported R6 rig. A local rig pack can supply custom meshes, with setup documented in the project README.</p>',
      '<h2>If playback is unavailable</h2>',
      '<p>Rendering requires WebGL 2 in Microsoft Edge WebView2. A cached model may only store a reference to an animation, or it may contain a format the timeline cannot play. DGI shows available details for those files. Browsing and playing local previews works without a Roblox Open Cloud API key.</p>',
    ].join('\n'),
  },
  {
    slug: 'restore-cached-assets',
    title: 'How to restore original cached assets in DGI',
    link: 'Restore an original cached asset',
    description: 'Undo an asset replacement in DGI Cache Studio, recover the first original backup, and understand folder scope and restore conflicts.',
    image: 'dgi-changes.webp',
    alt: 'DGI Changes page with individual Restore buttons, Restore all, and a decal preview',
    intro: 'Use the Changes page to undo a replacement you made in DGI. Each replacement keeps the first saved original so you can restore it after later replacements.',
    body: [
      '<h2>Restore one asset or the whole folder</h2>',
      '<ol><li>For a change to the Roblox cache, close Roblox Player before restoring.</li><li>In DGI, open <strong>Changes</strong> for the folder or editing workspace where you made the replacement.</li><li>Choose <strong>Restore</strong> beside the asset, or <strong>Restore all</strong> for the changes in the current folder. Review the confirmation and apply it.</li><li>For a restored Roblox cache entry, reopen Roblox to load the asset again.</li></ol>',
      '<h2>Choose the right location</h2>',
      '<p>The editing workspace and the Roblox cache hold separate copies. If you edited only a workspace copy, restore from that workspace. If you applied an edit to the Roblox cache, restore the corresponding cache entry in Changes.</p>',
      '<h2>What a restore conflict means</h2>',
      '<p>DGI checks the current bytes before it writes. If another process changed the asset after the replacement, DGI reports a conflict and protects the newer bytes. Rescan the folder and confirm you have the intended change selected.</p>',
    ].join('\n'),
  },
  {
    slug: 'fleasion-alternative',
    title: 'A Fleasion alternative for previewing Roblox assets',
    link: 'Fleasion alternative for asset previews',
    description: 'See what DGI Cache Studio can preview from the local Roblox cache on Windows and what to check before downloading.',
    image: 'dgi-preview.webp',
    alt: 'DGI Cache Studio showing image thumbnails and an interactive model preview',
    intro: 'DGI Cache Studio is a Windows alternative to Fleasion for browsing and previewing local Roblox assets. It builds a searchable library from the Roblox cache or a folder of files you choose.',
    body: [
      '<h2>What can I preview in DGI?</h2>',
      '<ul><li><strong>Images and textures.</strong> See thumbnails in the Library and open an image at a larger size.</li><li><strong>Audio.</strong> Play supported clips and seek through them in the inspector.</li><li><strong>Meshes and models.</strong> Rotate supported 3D previews.</li><li><strong>Animations.</strong> Play supported Roblox KeyframeSequence clips on an R6 or R15 rig and scrub the timeline.</li></ul>',
      '<h2>Where do the assets come from?</h2>',
      '<p>DGI reads files already on your PC. It can <a href="../roblox-cache-location/">find the Roblox cache</a> automatically or scan a folder of exported assets. The library shows supported files present at the time of the scan. Some cached objects only contain asset references.</p>',
      '<h2>What do I need to run it?</h2>',
      '<p>DGI uses Microsoft Edge WebView2 on Windows. Interactive 3D previews require WebGL 2. You can browse and preview local assets without a Roblox API key. The <a href="../../#download">Windows installer</a> is hosted on GitHub Releases and currently requires repository access.</p>',
    ].join('\n'),
  },
];

export function renderGuide(guide, origin) {
  const base = '../../';
  const canonical = origin && new URL('guides/' + guide.slug + '/', origin).href;
  const image = canonical ? new URL('assets/' + guide.image, origin).href : base + 'assets/' + guide.image;
  const title = guide.title + ' | DGI Cache Studio';
  const breadcrumbs = canonical && {
    '@context': 'https://schema.org',
    '@type': 'BreadcrumbList',
    itemListElement: [
      { '@type': 'ListItem', position: 1, name: 'DGI Cache Studio', item: origin },
      { '@type': 'ListItem', position: 2, name: guide.title, item: canonical },
    ],
  };
  const guideLinks = guides.filter(item => item.slug !== guide.slug)
    .map(item => '<li><a href="../' + item.slug + '/">' + item.link + '</a></li>').join('\n');

  const metadata = canonical
    ? `<link rel="canonical" href="${canonical}">
  <meta property="og:url" content="${canonical}">
  <script type="application/ld+json">${JSON.stringify(breadcrumbs)}</script>`
    : '<meta name="robots" content="noindex, nofollow">';

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="color-scheme" content="light dark">
  <title>${title}</title>
  <meta name="description" content="${guide.description}">
  ${metadata}
  <meta property="og:type" content="article">
  <meta property="og:site_name" content="DGI Cache Studio">
  <meta property="og:title" content="${title}">
  <meta property="og:description" content="${guide.description}">
  <meta property="og:image" content="${image}">
  <meta property="og:image:alt" content="${guide.alt}">
  <meta name="twitter:card" content="summary_large_image">
  <link rel="icon" type="image/png" sizes="128x128" href="${base}favicon.png">
  <link rel="stylesheet" href="${base}design-system.css">
  <link rel="stylesheet" href="${base}styles.css">
  <script src="${base}theme.js"></script>
</head>
<body>
  <a class="button button--primary skip-link" href="#main">Skip to content</a>
  <header class="site-header">
    <div class="site-width header-layout">
      <a class="wordmark brand-wordmark" href="${base}" aria-label="DGI Cache Studio home">DGI</a>
      <nav class="nav-links" aria-label="Main navigation">
        <a class="button button--ghost" href="${base}#guides">Guides</a>
        <a class="button button--ghost" href="${base}#download">Download</a>
      </nav>
    </div>
  </header>
  <main id="main" class="site-width guide-page" tabindex="-1">
    <article>
      <h1>${guide.title}</h1>
      <p class="guide-intro">${guide.intro}</p>
      <img src="${base}assets/${guide.image}" alt="${guide.alt}" width="1320" height="760">
      ${guide.body}
    </article>
    <nav class="guide-related" aria-label="Related guides">
      <h2>Related guides</h2>
      <ul>${guideLinks}</ul>
    </nav>
  </main>
</body>
</html>
`;
}
