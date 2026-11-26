import '@fontsource-variable/plus-jakarta-sans';
import './styles.css';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { button, icon } from './ui';
import { assetCategories, createAssetFilters, filterAssets, noCategoriesSelected, type AssetCategory } from './asset-filters';
import { createAssetSearch } from './asset-search';
import { mountAudio } from './audio-preview';

type AssetKind = 'image' | 'animation' | 'audio' | 'mesh' | 'reference';
type View = 'library' | 'changes' | 'settings';
type Theme = 'light' | 'dark';
type KeySource = 'manual' | 'process' | 'windows_user' | 'windows_system';
const keySourceLabels: Record<KeySource, string> = {
  manual: 'Key set for this session',
  process: 'Loaded from environment',
  windows_user: 'Loaded from Windows user variables',
  windows_system: 'Loaded from Windows system variables',
};
const previewCommands: Record<AssetKind, string> = {
  image: 'image_preview', animation: 'animation_preview', audio: 'audio_preview',
  mesh: 'mesh_preview', reference: 'model_preview',
};

interface Asset {
  id: string;
  name: string;
  defaultName?: string;
  customName?: string;
  sourceAssetId?: string;
  path: string;
  kind: AssetKind;
  category: AssetCategory;
  mime: string;
  size: number;
  modified: number;
  checksum: string;
  changed: boolean;
  previewOnly: boolean;
  databaseEntry?: boolean;
  cacheEntry?: boolean;
}

interface Change {
  id: string;
  name: string;
  path: string;
  originalChecksum: string;
  currentChecksum: string;
  changedAt: number;
}

interface Snapshot {
  folder: string | null;
  detected: boolean;
  workspace: boolean;
  cacheFolder?: string | null;
  canClearCache?: boolean;
  assets: Asset[];
  changes: Change[];
}

interface ScanUpdate {
  job: number;
  checked: number;
  total: number | null;
}

interface ClearedCache {
  snapshot: Snapshot;
  report: { removed: number; kept: number; failed: number; databaseRemoved: number; databaseError?: string | null };
  rescanError?: string | null;
}

interface WorkspaceAsset {
  snapshot: Snapshot;
  id: string;
}

function el<Tag extends keyof HTMLElementTagNameMap>(
  tag: Tag, className = '', ...children: (Node | string)[]
): HTMLElementTagNameMap[Tag] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  node.append(...children);
  return node;
}

function action(label: string, options: Parameters<typeof button>[1], run: () => void) {
  const node = button(label, options);
  node.addEventListener('click', run);
  return node;
}

function field(label: string, input: HTMLInputElement) {
  return el('label', 'field', el('span', 'field-label', label), input);
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 ** 2) return (bytes / 1024).toFixed(1) + ' KB';
  return (bytes / 1024 ** 2).toFixed(1) + ' MB';
}

const date = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });
const themePreference = 'dgi.theme';
let theme: Theme = 'dark';
try {
  if (localStorage.getItem(themePreference) === 'light') theme = 'light';
} catch {
  // The current session can still switch themes.
}
document.documentElement.dataset.theme = theme;

let snapshot: Snapshot = { folder: null, detected: false, workspace: false, assets: [], changes: [] };
let view: View = 'library';
let selectedId: string | null = null;
const pageSize = 120;
const refreshInterval = 3_000;
let shown = pageSize;
let keySource: KeySource | null = null;
let busy = false;
let refreshing = false;
let sequence = 0;
let closeInlinePanel: ((restoreFocus?: boolean) => void) | undefined;
let disposePreview: (() => void) | undefined;
let refreshLibraryResults: ((updateFilters?: boolean) => void) | undefined;
let updateLibrarySelection: (() => void) | undefined;
let observer: IntersectionObserver | undefined;
const thumbUrls = new Map<string, string>();
const thumbQueue: { image: HTMLImageElement; asset: Asset }[] = [];
const thumbPending = new WeakSet<HTMLImageElement>();
let activeThumbs = 0;

const content = el('main', 'main-content');
content.id = 'main-content';
content.tabIndex = -1;
const status = el('div', 'status-line');
status.setAttribute('role', 'status');
status.setAttribute('aria-live', 'polite');
const scanBar = el('progress');
scanBar.setAttribute('aria-label', 'Cache scan progress');
const scanCount = el('span', 'scan-count');
scanCount.setAttribute('aria-hidden', 'true');
const scanMetrics = el('span', 'scan-metrics', scanBar, scanCount);
const location = el('span', 'truncate');
const locationBar = el('div', 'sidebar-location', icon('hard-drive'), location);
const assetSearch = createAssetSearch(updateSearch, selectLibraryAsset);
const { element: search, input: searchInput } = assetSearch;

function updateSearch() {
  shown = pageSize;
  if (refreshLibraryResults) refreshLibraryResults();
  else assetSearch.setMatches([]);
}

function selectLibraryAsset(id: string) {
  if (selectedId === id) return;
  selectedId = id;
  if (updateLibrarySelection) updateLibrarySelection();
  else render();
  revealInspector();
}

const choose = action('Choose folder', { icon: 'folder-open', variant: 'secondary' }, () => {
  void operate('Waiting for a folder…', async job => {
    const result = await invoke<Snapshot | null>('choose_cache', { job });
    if (!result) return '';
    selectedId = null;
    shown = pageSize;
    searchInput.value = '';
    view = result.workspace ? 'changes' : 'library';
    snapshot = result;
    return result.workspace ? 'Editing workspace opened.' : scanMessage();
  });
});
const scan = action('Scan cache', { icon: 'scan', variant: 'primary' }, () => {
  void operate('Scanning cache…', async job => {
    showScanProgress({ job, checked: 0, total: null });
    snapshot = await invoke<Snapshot>('scan', { job });
    return scanMessage();
  });
});
const detect = action('Find Roblox cache', { icon: 'scan', variant: 'primary' }, () => {
  void operate('Looking for the Roblox cache…', async job => {
    showScanProgress({ job, checked: 0, total: null });
    const result = await invoke<Snapshot>('detect_cache', { job });
    if (!result.folder) throw new Error('No Roblox cache found. Choose a folder to scan.');
    snapshot = result;
    selectedId = null;
    shown = pageSize;
    searchInput.value = '';
    if (view === 'settings') view = 'library';
    return scanMessage();
  });
});
const clearCache = action('Clear cache', { icon: 'trash', variant: 'danger' }, () => {
  const folder = snapshot.folder;
  if (!folder || !snapshot.canClearCache || busy) return;
  confirmAction(
    clearCache,
    'Clear Roblox cache?',
    'This removes cached files and responses from the selected Roblox cache. DGI keeps active replacements, backups, and the editing workspace.',
    'Clear cache', 'trash', () => {
      void operate('Clearing cache…', async job => {
        const result = await invoke<ClearedCache>('clear_cache', { folder, job });
        for (const url of thumbUrls.values()) URL.revokeObjectURL(url);
        thumbUrls.clear();
        snapshot = result.snapshot;
        if (!snapshot.assets.some(asset => asset.id === selectedId)) selectedId = null;
        shown = pageSize;
        const { removed, kept, failed, databaseRemoved, databaseError } = result.report;
        const databaseCount = snapshot.assets.filter(asset => asset.databaseEntry).length;
        const summary = [
          removed.toLocaleString() + (removed === 1 ? ' file removed.' : ' files removed.'),
          databaseRemoved ? databaseRemoved.toLocaleString() + (databaseRemoved === 1
            ? ' database response cleared.' : ' database responses cleared.') : '',
          kept ? kept.toLocaleString() + (kept === 1 ? ' file kept.' : ' files kept.') : '',
          failed ? failed.toLocaleString() + (failed === 1 ? ' file could not be removed.' : ' files could not be removed.') : '',
          databaseCount ? databaseCount.toLocaleString() + (databaseCount === 1
            ? ' asset remains in Roblox’s cache database.' : ' assets remain in Roblox’s cache database.') : '',
          databaseError ? 'Could not clear Roblox’s cache database. ' + databaseError : '',
          result.rescanError ? 'Could not rescan the cache. ' + result.rescanError : '',
        ].filter(Boolean).join(' ');
        if (failed || databaseError || result.rescanError) {
          render();
          throw new Error(summary);
        }
        return summary;
      });
    },
  );
});
clearCache.setAttribute('aria-expanded', 'false');
const nav = el('nav', 'nav');
nav.setAttribute('aria-label', 'Workspace');
const views: [View, string, string][] = [
  ['library', 'Library', 'grid'],
  ['changes', 'Changes', 'history'],
  ['settings', 'Settings', 'settings'],
];
const navButtons = new Map<View, HTMLButtonElement>();
for (const [id, label, glyph] of views) {
  const item = action(label, { icon: glyph, variant: 'ghost' }, () => {
    if (id === 'library' && snapshot.workspace) {
      void operate('Returning to the cache library…', async job => {
        showScanProgress({ job, checked: 0, total: null });
        snapshot = await invoke<Snapshot>('open_library', { job });
        selectedId = null;
        shown = pageSize;
        searchInput.value = '';
        view = 'library';
        return scanMessage();
      });
      return;
    }
    if (view !== id) {
      view = id;
      render();
      content.focus();
    }
  });
  navButtons.set(id, item);
  nav.append(item);
}
const sidebar = el('aside', 'sidebar', nav, locationBar);
const brand = el('span', 'brand-wordmark', 'DGI');
brand.setAttribute('aria-hidden', 'true');
const wordmark = el('div', 'wordmark', brand, el('span', 'sr-only', 'DGI Cache Studio'));
const dragSpace = el('div', 'caption-space');
const topbar = el('header', 'topbar', wordmark, search, dragSpace);
for (const region of [topbar, wordmark, brand, dragSpace]) {
  region.setAttribute('data-tauri-drag-region', '');
}
const native = '__TAURI_INTERNALS__' in window;
if (native && /Windows/i.test(navigator.userAgent)) topbar.classList.add('has-native-caption');
function applyNativeTheme() {
  if (native) void import('@tauri-apps/api/window')
    .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(theme))
    .catch(() => { /* The app theme also works in a browser preview. */ });
}
applyNativeTheme();
const workspace = el('div', 'workspace', content, status);
document.getElementById('app')?.replaceChildren(el('div', 'app-shell', topbar, sidebar, workspace));
document.addEventListener('contextmenu', event => event.preventDefault());

function announce(message: string, error = false) {
  status.classList.toggle('is-error', error);
  status.classList.toggle('is-busy', busy);
  status.replaceChildren();
  if (message) status.append(icon(error ? 'alert' : busy ? 'loader' : 'check-circle'), message);
  status.hidden = !message;
}

function showScanProgress({ job, checked, total }: ScanUpdate) {
  if (!busy || job !== sequence) return;
  if (scanMetrics.parentElement !== status) {
    announce('Scanning cache…');
    status.append(scanMetrics);
  }
  const emptyTitle = document.getElementById('cache-empty-title');
  if (emptyTitle) emptyTitle.textContent = total === null ? 'Finding cached files…' : 'Scanning cached files…';
  if (total === null) {
    scanBar.removeAttribute('value');
    scanCount.textContent = checked ? checked.toLocaleString() + ' files found' : 'Finding files…';
    scanBar.setAttribute('aria-valuetext', scanCount.textContent);
  } else {
    scanBar.max = Math.max(1, total);
    scanBar.value = total ? checked : 1;
    scanCount.textContent = checked.toLocaleString() + ' / ' + total.toLocaleString() + ' files';
    scanBar.setAttribute('aria-valuetext', scanCount.textContent);
  }
}

function describeError(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

async function operate(message: string, work: (job: number) => Promise<string>): Promise<boolean> {
  if (busy) return false;
  closeInlinePanel?.(false);
  busy = true;
  const job = ++sequence;
  syncBusy();
  announce(message);
  let result: string;
  let failed = false;
  try {
    result = await work(job);
  } catch (error) {
    failed = true;
    result = describeError(error);
  }
  busy = false;
  if (failed) syncBusy();
  else render();
  announce(result, failed);
  return !failed;
}

function syncBusy() {
  choose.disabled = busy;
  detect.disabled = busy;
  scan.disabled = busy || !snapshot.folder;
  clearCache.disabled = busy || !snapshot.canClearCache;
  const emptyTitle = document.getElementById('cache-empty-title');
  if (emptyTitle && !busy) emptyTitle.textContent = 'No cache folder selected';
  for (const control of content.querySelectorAll<HTMLInputElement | HTMLButtonElement>('[data-operation]')) {
    control.disabled = busy;
  }
}

function operationControl<T extends HTMLElement>(control: T): T {
  control.dataset.operation = '';
  return control;
}

function scanMessage() {
  const count = snapshot.assets.length;
  if (!snapshot.folder) return '';
  const result = count + ' supported ' + (count === 1 ? 'asset' : 'assets') + ' found.';
  return (snapshot.detected ? 'Roblox cache detected. ' : 'Scan complete. ') + result;
}

function selectForChanges(id: string) {
  if (view === 'changes' && selectedId === id) return;
  view = 'changes';
  selectedId = id;
  render();
  revealInspector();
}

function revealInspector() {
  const panel = content.querySelector('.inspector');
  panel?.classList.add('inspector-enter');
  panel?.addEventListener('animationend', () => panel.classList.remove('inspector-enter'), { once: true });
  panel?.querySelector<HTMLElement>('h2')?.focus();
}

function openWorkspaceAsset(asset: WorkspaceAsset) {
  snapshot = asset.snapshot;
  selectedId = asset.id;
  view = 'changes';
}

function applyWorkspaceEdit(id: string) {
  void operate('Finding the original cache entry and applying your edit…', async () => {
    openWorkspaceAsset(await invoke<WorkspaceAsset>('apply_workspace_asset', { id }));
    return 'Workspace edit applied and verified in the Roblox cache. Reopen Roblox to load it.';
  });
}

function openWorkspace() {
  void operate('Opening editing workspace…', async () => {
    snapshot = await invoke<Snapshot>('open_workspace');
    selectedId = null;
    return 'Editing workspace opened.';
  });
}

function importAsset() {
  void operate('Choose an asset to import…', async () => {
    const imported = await invoke<WorkspaceAsset | null>('import_asset');
    if (!imported) return '';
    openWorkspaceAsset(imported);
    return 'Asset added to the editing workspace.';
  });
}

function pickAsset(trigger: HTMLButtonElement) {
  const panel = el('section', 'panel asset-picker');
  panel.id = 'asset-picker';
  const title = el('h2', '', 'Add asset');
  title.id = 'asset-picker-title';
  panel.setAttribute('aria-labelledby', title.id);
  const input = el('input');
  input.type = 'search';
  input.placeholder = 'Search names, IDs, and types';
  input.setAttribute('aria-label', 'Search candidate assets');
  const results = el('div', 'asset-picker-results');
  const count = el('p', 'asset-picker-count muted');
  count.setAttribute('role', 'status');
  const cancel = action('Cancel', { variant: 'ghost' }, () => closeInlinePanel?.());
  const importFile = action('Import local file', { variant: 'secondary', icon: 'upload' }, () => {
    importAsset();
  });
  const take = 60;
  let limit = take;

  function showResults() {
    const matches = filterAssets(snapshot.assets, input.value);
    count.textContent = matches.length ? matches.length.toLocaleString() + ' available' :
      noCategoriesSelected() ? 'No asset types selected' : 'No matching assets';
    results.replaceChildren();
    for (const asset of matches.slice(0, limit)) {
      const item = button('', { variant: 'ghost', icon: assetCategories[asset.category].icon });
      item.classList.add('asset-picker-row');
      item.append(
        el('span', 'asset-picker-name', el('strong', 'truncate', asset.name || asset.path),
          el('span', 'muted truncate', asset.sourceAssetId ? 'Asset ID ' + asset.sourceAssetId : formatSize(asset.size))),
        el('span', 'badge', asset.kind === 'reference' || asset.kind === 'mesh' ? 'Preview only'
          : asset.previewOnly ? 'Workspace copy required' : assetTypeLabel(asset)));
      item.addEventListener('click', () => {
        closeInlinePanel?.();
        selectForChanges(asset.id);
      });
      results.append(item);
    }
    if (matches.length > limit) {
      results.append(action('Show more', { size: 'small', variant: 'secondary' }, () => {
        const firstNew = limit;
        limit += take;
        showResults();
        results.querySelectorAll<HTMLButtonElement>('.asset-picker-row')[firstNew]?.focus();
      }));
    }
  }

  input.addEventListener('input', () => { limit = take; showResults(); });
  const filters = createAssetFilters(snapshot.assets, () => { limit = take; showResults(); });
  panel.append(title, el('div', 'asset-picker-toolbar', el('div', 'search', icon('search'), input), filters), count, results,
    el('div', 'inline-actions', cancel, importFile));
  showResults();
  showInlinePanel(trigger, panel, input);
}

function heading(title: string, ...tools: HTMLElement[]) {
  return el('header', 'page-header', el('h1', '', title), el('div', 'toolbar', ...tools));
}

function empty(glyph: string, title: string, ...tools: HTMLElement[]) {
  return el('section', 'empty-state panel',
    el('div', 'empty-icon', icon(glyph)), el('h2', '', title), ...tools);
}

function assetThumbnail(asset: Asset) {
  const thumb = el('span', 'asset-thumb', icon(assetCategories[asset.category].icon));
  if (asset.kind === 'image') {
    const image = el('img');
    image.alt = '';
    image.hidden = true;
    image.addEventListener('load', () => { image.hidden = false; });
    const cached = thumbUrls.get(asset.id + ':' + asset.checksum);
    if (cached) image.src = cached;
    thumb.append(image);
    thumbItems.set(thumb, { image, asset });
  }
  return thumb;
}

let thumbItems = new WeakMap<Element, { image: HTMLImageElement; asset: Asset }>();

function observeThumbnails() {
  observer = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      observer?.unobserve(entry.target);
      const item = thumbItems.get(entry.target);
      if (item) thumbQueue.push(item);
    }
    pumpThumbnails();
  }, { rootMargin: '100px' });
  for (const thumb of content.querySelectorAll('.asset-thumb')) {
    const item = thumbItems.get(thumb);
    if (item && !item.image.hasAttribute('src') && !thumbPending.has(item.image)) observer.observe(thumb);
  }
}

function pumpThumbnails() {
  while (activeThumbs < 4 && thumbQueue.length) {
    const item = thumbQueue.shift();
    if (!item?.image.isConnected) continue;
    const { image, asset } = item;
    if (image.hasAttribute('src') || thumbPending.has(image)) continue;
    const key = asset.id + ':' + asset.checksum;
    const cached = thumbUrls.get(key);
    if (cached) {
      image.src = cached;
      continue;
    }
    activeThumbs++;
    thumbPending.add(image);
    void invoke<ArrayBuffer>('thumbnail', { id: asset.id }).then(bytes => {
      if (!image.isConnected) return;
      const existing = thumbUrls.get(key);
      const url = existing ?? URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
      if (!existing) thumbUrls.set(key, url);
      if (thumbUrls.size > 160) {
        const oldest = thumbUrls.keys().next().value;
        if (oldest) {
          const oldUrl = thumbUrls.get(oldest);
          if (oldUrl) URL.revokeObjectURL(oldUrl);
          thumbUrls.delete(oldest);
        }
      }
      image.src = url;
    }).catch(() => {
      // The asset icon remains available when a thumbnail cannot be decoded.
    }).finally(() => {
      thumbPending.delete(image);
      activeThumbs--;
      pumpThumbnails();
    });
  }
}

function renderLibrary() {
  const header = snapshot.folder
    ? heading('Library', ...(snapshot.canClearCache ? [clearCache] : []), choose, scan)
    : heading('Library');
  if (!snapshot.folder) {
    assetSearch.setMatches([]);
    const welcome = empty('folder-open', 'No cache folder selected', el('div', 'toolbar', detect, choose));
    welcome.querySelector('h2')!.id = 'cache-empty-title';
    content.append(header, welcome);
    return;
  }

  const selected = snapshot.assets.find(asset => asset.id === selectedId);
  const count = el('span', 'asset-count muted');
  count.setAttribute('role', 'status');
  const results = el('div');
  let filters = makeFilters();
  const toolbar = el('div', 'library-toolbar', filters, count);
  const listing = el('div', 'asset-results', toolbar, results);
  const grid = el('div', 'asset-grid');
  const cards = new Map<string, { asset: Asset; card: HTMLButtonElement }>();
  const area = el('div', selected ? 'asset-workspace has-selection' : 'asset-workspace', listing);
  if (selected) area.append(inspector(selected));
  content.append(header, area);
  updateLibrarySelection = replaceSelection;
  refreshLibraryResults = showResults;
  showResults();

  function replaceSelection() {
    disposePreview?.();
    disposePreview = undefined;
    area.querySelector('.inspector')?.remove();
    const asset = snapshot.assets.find(item => item.id === selectedId);
    area.classList.toggle('has-selection', !!asset);
    if (asset) area.append(inspector(asset));
    for (const [id, { card }] of cards) card.setAttribute('aria-pressed', String(id === selectedId));
    syncBusy();
  }

  function makeFilters() {
    return createAssetFilters(snapshot.assets, () => { shown = pageSize; showResults(); });
  }

  function showResults(updateFilters = false) {
    if (updateFilters) {
      const next = makeFilters();
      filters.replaceWith(next);
      filters = next;
    }
    const assets = snapshot.assets;
    const visible = filterAssets(assets, searchInput.value);
    assetSearch.setMatches(visible);
    count.textContent = visible.length === assets.length
      ? assets.length.toLocaleString() + (assets.length === 1 ? ' asset' : ' assets')
      : visible.length.toLocaleString() + ' of ' + assets.length.toLocaleString() + ' assets';
    observer?.disconnect();
    thumbQueue.length = 0;
    results.querySelector('.load-more')?.remove();
    const displayed = visible.slice(0, shown);
    const ids = new Set(displayed.map(asset => asset.id));
    for (const [id, entry] of cards) {
      if (ids.has(id)) continue;
      entry.card.remove();
      cards.delete(id);
    }
    if (!visible.length) {
      const label = !assets.length ? 'No supported assets found'
        : noCategoriesSelected() ? 'No asset types selected' : 'No matching assets';
      results.replaceChildren(empty(assets.length ? 'search' : 'inbox', label));
      return;
    }

    displayed.forEach((asset, index) => {
      let entry = cards.get(asset.id);
      if (!entry || !sameAsset(entry.asset, asset)) {
        entry?.card.remove();
        entry = { asset, card: makeCard(asset) };
        cards.set(asset.id, entry);
      }
      if (grid.children[index] !== entry.card) grid.insertBefore(entry.card, grid.children[index] ?? null);
    });
    if (!grid.isConnected) results.replaceChildren(grid);
    if (visible.length > shown) {
      const more = action('Load more', { size: 'small', variant: 'secondary' }, () => {
        const next = visible[shown];
        shown += pageSize;
        showResults();
        gridFocus(next.id);
      });
      results.append(el('div', 'load-more', more));
    }
    observeThumbnails();
  }

  function makeCard(asset: Asset) {
    const card = el('button', 'asset-card', assetThumbnail(asset),
      el('span', 'asset-card-text',
        el('strong', 'truncate', asset.name || asset.path),
        el('span', 'asset-meta', assetTypeLabel(asset) + ' · ' + formatSize(asset.size))));
    card.type = 'button';
    card.title = asset.name;
    card.dataset.assetId = asset.id;
    card.dataset.assetCategory = asset.category;
    card.setAttribute('aria-pressed', String(selectedId === asset.id));
    if (asset.changed) card.append(el('span', 'badge asset-badge', 'Changed'));
    card.addEventListener('click', () => selectLibraryAsset(asset.id));
    return card;
  }

  function gridFocus(id: string) {
    content.querySelector<HTMLElement>('[data-asset-id="' + CSS.escape(id) + '"]')?.focus();
  }
}

function assetTypeLabel(asset: Asset) {
  return assetCategories[asset.category].singular;
}

function inspector(asset: Asset) {
  const title = el('h2', 'truncate', asset.name || asset.path);
  title.tabIndex = -1;
  const close = action('', { icon: 'x', size: 'icon', ariaLabel: 'Close preview', variant: 'ghost' }, () => {
    selectedId = null;
    if (updateLibrarySelection) updateLibrarySelection();
    else render();
    content.querySelector<HTMLElement>('[data-asset-id="' + asset.id + '"]')?.focus({ preventScroll: true });
  });
  const stage = el('div', 'preview-stage', el('p', 'muted', 'Loading preview…'));
  if (asset.kind === 'audio') stage.classList.add('audio-stage');
  let disposed = false;
  let cleanup: (() => void) | undefined;
  disposePreview = () => { disposed = true; cleanup?.(); };
  function showPreviewDetails(reason: string) {
    if (disposed) return;
    stage.replaceChildren(el('div', 'preview-details', icon(assetCategories[asset.category].icon),
      el('strong', '', assetTypeLabel(asset)), el('span', 'muted', formatSize(asset.size)),
      el('span', 'muted', reason)));
  }

  void invoke<ArrayBuffer>(previewCommands[asset.kind], { id: asset.id }).then(async bytes => {
    if (disposed) return;
    if (asset.kind === 'animation') {
      const [{ mountRig }, { loadStudioMeshes }] = await Promise.all([import('./rig'), import('./rig-mesh')]);
      const geometry = await loadStudioMeshes();
      if (!disposed) cleanup = mountRig(stage, new Uint8Array(bytes), geometry);
    } else if (asset.kind === 'mesh') {
      const { mountMesh } = await import('./mesh-view');
      if (!disposed) cleanup = mountMesh(stage, bytes);
    } else if (asset.kind === 'reference') {
      const { mountModel } = await import('./model-view');
      if (!disposed) cleanup = mountModel(stage, bytes);
    } else if (asset.kind === 'audio') {
      cleanup = mountAudio(stage, bytes, asset.mime, () => showPreviewDetails('This audio file could not be played.'));
    } else {
      stage.replaceChildren();
      const image = el('img', 'preview-image');
      const type = asset.mime === 'image/ktx2' ? 'image/png' : asset.mime;
      const url = URL.createObjectURL(new Blob([bytes], { type }));
      cleanup = () => URL.revokeObjectURL(url);
      image.src = url;
      image.alt = 'Preview of ' + (asset.name || asset.path);
      image.addEventListener('error', () => showPreviewDetails('This image could not be displayed.'), { once: true });
      stage.append(image);
    }
  }).catch(async error => {
    if (!disposed && asset.kind === 'animation') {
      try {
        const [{ mountModel }, bytes] = await Promise.all([
          import('./model-view'), invoke<ArrayBuffer>('model_preview', { id: asset.id }),
        ]);
        if (!disposed) cleanup = mountModel(stage, bytes, describeError(error));
        return;
      } catch {
        // Keep the original playback error if the object preview also fails.
      }
    }
    showPreviewDetails(describeError(error));
  });

  const name = el('input');
  name.value = asset.customName ?? '';
  name.maxLength = 120;
  name.placeholder = asset.defaultName ?? asset.name;
  const save = operationControl(button('Save', { size: 'small', variant: 'secondary', type: 'submit' }));
  const nameActions = el('div', 'field-actions', field('Custom name', name), save);
  const naming = el('form', 'inspector-section', nameActions);
  if (asset.customName) {
    nameActions.append(operationControl(action('Reset name', { size: 'small', variant: 'ghost' }, () => {
      name.value = '';
      naming.requestSubmit();
    })));
  }
  naming.addEventListener('submit', event => {
    event.preventDefault();
    void operate('Saving name…', async () => {
      snapshot = await invoke<Snapshot>('rename', { id: asset.id, name: name.value });
      return 'Asset name saved.';
    });
  });

  const folder = snapshot.folder;
  const separator = folder?.includes('\\') ? '\\' : '/';
  const filePath = folder ? folder.replace(/[\\/]$/, '') + separator + asset.path.replace(/\//g, separator) : asset.path;
  const meta = el('div', 'inspector-meta', el('span', 'file-path muted', asset.databaseEntry && folder ? folder.replace(/[\\/]rbx-storage[\\/]?$/, '') + separator + asset.path.replace(/\//g, separator) : filePath),
    el('span', 'muted', formatSize(asset.size) + ' · ' + assetTypeLabel(asset)));
  if (asset.sourceAssetId) meta.append(el('span', 'muted', 'Asset ID ' + asset.sourceAssetId));
  if (asset.changed) meta.append(el('span', 'badge', 'Changed'));
  if (snapshot.workspace) meta.append(el('span', 'badge', 'DGI workspace file'));
  if (asset.databaseEntry) meta.append(el('span', 'badge', 'Roblox cache database'));
  if (asset.previewOnly) meta.append(el('span', 'badge', 'Preview only'));
  const panel = el('aside', 'inspector panel', el('div', 'inspector-heading', title, close), stage, meta, naming);
  if (asset.kind === 'reference' || asset.kind === 'mesh') return panel;
  if (asset.previewOnly) {
    const copy = operationControl(action('Copy to DGI workspace', { icon: 'file', variant: 'primary' }, () => {
      void operate('Creating a workspace copy…', async () => {
        openWorkspaceAsset(await invoke<WorkspaceAsset>('copy_asset', { id: asset.id }));
        return 'Workspace copy created. Roblox continues to use its own cache.';
      });
    }));
    panel.append(el('div', 'inspector-section',
      el('small', 'muted', 'This cache entry is read-only in DGI. Copy it to edit a separate file.'), copy));
    return panel;
  }

  function replacementMessage() {
    return snapshot.workspace ? 'Replacement verified in the DGI workspace.' : asset.cacheEntry ? 'Replacement verified in the Roblox cache. Reopen Roblox to load it.' : 'Replacement verified in the selected folder.';
  }
  const local = operationControl(action('Choose local file', { icon: 'upload', variant: 'secondary' }, () => {
    void operate(snapshot.workspace ? 'Choose a replacement for the workspace file…' : 'Choose a replacement for the selected file…', async () => {
      const result = await invoke<Snapshot | null>('replace_local', { id: asset.id });
      if (!result) return '';
      snapshot = result;
      return replacementMessage();
    });
  }));
  const assetId = el('input');
  assetId.inputMode = 'numeric';
  assetId.pattern = '[0-9]+';
  assetId.maxLength = 20;
  assetId.placeholder = 'e.g. 123456789';
  assetId.required = true;
  const remote = operationControl(button('Replace', { icon: 'download', variant: 'primary', type: 'submit' }));
  const form = el('form', 'field-actions', field('Roblox asset ID', assetId), remote);
  form.addEventListener('submit', event => {
    event.preventDefault();
    void operate('Requesting asset from Roblox…', async job => {
      snapshot = await invoke<Snapshot>('replace_remote', { id: asset.id, assetId: assetId.value, job });
      return replacementMessage();
    });
  });
  const replacement = el('section', 'inspector-section',
    el('h3', '', snapshot.workspace ? 'Replace workspace file' : asset.cacheEntry ? 'Replace Roblox cache' : 'Replace file'),
    ...(snapshot.workspace ? [el('small', 'muted', 'Roblox continues to use its own cache.')] : asset.cacheEntry ? [el('small', 'muted', 'Close Roblox Player, apply the replacement, then join again. The original is backed up.')] : []), form, local);
  if (snapshot.workspace && asset.changed) {
    const apply = operationControl(action('Apply to Roblox cache', { icon: 'check', variant: 'primary' }, () => {
      applyWorkspaceEdit(asset.id);
    }));
    replacement.append(el('small', 'muted', 'Close Roblox Player before applying this edit to its original cache entry.'), apply);
  }
  if (!keySource) {
    const settings = action('Add API key in Settings', { variant: 'ghost', size: 'small', icon: 'key' }, () => {
      view = 'settings';
      render();
      content.focus();
    });
    replacement.append(settings);
  }
  panel.append(replacement);
  return panel;
}

function showInlinePanel(trigger: HTMLButtonElement, panel: HTMLElement, focus: HTMLElement) {
  const anchor = trigger.closest('.page-header, .change-row');
  if (!anchor) return;
  if (trigger.getAttribute('aria-expanded') === 'true') {
    closeInlinePanel?.();
    return;
  }
  closeInlinePanel?.(false);
  trigger.setAttribute('aria-expanded', 'true');
  trigger.setAttribute('aria-controls', panel.id);
  function close(restoreFocus = true) {
    panel.remove();
    trigger.setAttribute('aria-expanded', 'false');
    trigger.removeAttribute('aria-controls');
    closeInlinePanel = undefined;
    if (restoreFocus && trigger.isConnected) trigger.focus();
  }
  closeInlinePanel = close;
  panel.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    event.preventDefault();
    close();
  });
  if (anchor.classList.contains('change-row')) anchor.append(panel);
  else anchor.after(panel);
  focus.focus();
}

function confirmRestore(trigger: HTMLButtonElement, title: string, description: string, run: () => void) {
  confirmAction(trigger, title, description, 'Restore', 'rotate-ccw', run);
}

function confirmAction(trigger: HTMLButtonElement, title: string, description: string, label: string, glyph: string, run: () => void) {
  const heading = el('h2', '', title);
  heading.id = 'confirm-title';
  const cancel = action('Cancel', { variant: 'secondary', size: 'small' }, () => closeInlinePanel?.());
  const confirm = action(label, { icon: glyph, variant: 'danger', size: 'small' }, () => {
    closeInlinePanel?.();
    run();
  });
  const panel = el('section', 'notice inline-confirmation',
    el('div', 'confirmation-copy', heading, el('p', 'muted', description)),
    el('div', 'inline-actions', cancel, confirm));
  panel.id = 'inline-confirmation';
  panel.setAttribute('aria-labelledby', heading.id);
  showInlinePanel(trigger, panel, cancel);
}

function renderChanges() {
  const selected = snapshot.assets.find(asset => asset.id === selectedId);
  const add = operationControl(button('Add asset', { icon: 'file', variant: 'primary' }));
  add.setAttribute('aria-expanded', 'false');
  add.addEventListener('click', () => pickAsset(add));
  const workspace = snapshot.workspace
    ? detect
    : operationControl(action('Editing workspace', { icon: 'folder-open', variant: 'secondary' }, openWorkspace));
  const count = snapshot.changes.length;
  const all = operationControl(action('Restore all', { icon: 'rotate-ccw', variant: 'danger' }, () => {
    confirmRestore(all, 'Restore all originals?', 'Restore the first saved copy of each changed asset in this folder.', () => {
      void operate('Restoring originals and verifying checksums…', async () => {
        try {
          snapshot = await invoke<Snapshot>('restore_all');
        } catch (error) {
          const updated = await invoke<Snapshot>('scan').catch(() => null);
          if (updated) {
            snapshot = updated;
            render();
          }
          throw error;
        }
        return snapshot.workspace ? 'All workspace originals restored and verified.' : 'All originals in the selected folder restored and verified.';
      });
    });
  }));
  all.setAttribute('aria-expanded', 'false');
  content.append(heading(snapshot.workspace ? 'Workspace changes' : 'Changes', workspace, add, ...(count ? [all] : [])));
  const area = el('div', selected ? 'asset-workspace has-selection changes-workspace' : 'asset-workspace changes-workspace');
  content.append(area);
  if (!count) {
    area.append(el('div', '', empty('history', 'No changes to restore')));
    if (selected) area.append(inspector(selected));
    return;
  }
  const list = el('div', 'changes-list panel');
  for (const change of snapshot.changes) {
    const name = change.name || change.path;
    const restore = operationControl(action('Restore', { icon: 'rotate-ccw', size: 'small', variant: 'secondary' }, () => {
      confirmRestore(restore, 'Restore original?', 'Restore the first saved copy of ' + name + '.', () => {
        void operate('Restoring original and verifying checksum…', async () => {
          snapshot = await invoke<Snapshot>('restore', { id: change.id });
          return snapshot.workspace ? 'Workspace original restored and verified.' : 'Original in the selected folder restored and verified.';
        });
      });
    }));
    restore.setAttribute('aria-expanded', 'false');
    const times = el('span', 'muted change-time', change.changedAt > 0 ? date.format(change.changedAt) : 'Backup available');
    const edit = action('Edit', { icon: 'edit', size: 'small', variant: 'ghost', ariaLabel: 'Edit ' + name }, () => selectForChanges(change.id));
    const canEdit = snapshot.assets.some(asset => asset.id === change.id);
    const apply = snapshot.workspace && canEdit ? operationControl(action('Apply', {
      icon: 'check', size: 'small', variant: 'primary', ariaLabel: 'Apply ' + name + ' to Roblox cache',
    }, () => applyWorkspaceEdit(change.id))) : null;
    list.append(el('div', 'change-row',
      el('span', 'change-icon', icon('file')),
      el('div', 'change-file', el('strong', 'truncate', name), el('span', 'muted file-path', change.path)),
      times, el('div', 'change-actions', ...(apply ? [apply] : []), ...(canEdit ? [edit] : []), restore)));
  }
  area.append(list);
  if (selected) area.append(inspector(selected));
}

function renderSettings() {
  const key = el('input');
  key.type = 'password';
  key.autocomplete = 'off';
  key.spellcheck = false;
  key.required = true;
  key.placeholder = keySource ? 'API key loaded · enter a different key to override' : 'Enter your Open Cloud API key';
  const save = operationControl(button('Set API key', { icon: 'check', variant: 'primary', type: 'submit' }));
  const clear = operationControl(action('Clear key', { variant: 'ghost', size: 'small' }, () => {
    void operate('Clearing key…', async () => {
      keySource = await invoke<KeySource | null>('set_api_key', { key: '' });
      return 'API key cleared for this session.';
    });
  }));
  const form = el('form', 'field-actions', field('API key', key), save);
  form.addEventListener('submit', event => {
    event.preventDefault();
    const entered = key.value;
    key.value = '';
    void operate('Setting key…', async () => {
      keySource = await invoke<KeySource | null>('set_api_key', { key: entered });
      return 'API key set for this session.';
    });
  });
  const detectKey = operationControl(action('Check environment', { icon: 'scan', variant: 'secondary', size: 'small' }, () => {
    void operate('Checking environment variables…', async () => {
      const detected = await invoke<KeySource | null>('detect_api_key');
      if (!detected) throw new Error('No Roblox Open Cloud API key was found in the environment.');
      keySource = detected;
      return keySourceLabels[detected] + '.';
    });
  }));
  const themes = el('div', 'segment theme-options');
  themes.setAttribute('role', 'group');
  themes.setAttribute('aria-label', 'Color theme');
  const themeButtons = new Map<Theme, HTMLButtonElement>();
  for (const mode of ['light', 'dark'] as const) {
    const control = action(mode === 'light' ? 'Light' : 'Dark', { size: 'small', variant: 'ghost' }, () => {
      theme = mode;
      document.documentElement.dataset.theme = mode;
      try { localStorage.setItem(themePreference, mode); } catch { /* Apply for this session. */ }
      applyNativeTheme();
      for (const [name, item] of themeButtons) item.setAttribute('aria-pressed', String(name === mode));
    });
    control.setAttribute('aria-pressed', String(theme === mode));
    themes.append(control);
    themeButtons.set(mode, control);
  }
  content.append(heading('Settings'),
    el('section', 'settings-panel panel', el('h2', '', 'Appearance'), themes),
    el('section', 'settings-panel panel',
      el('h2', '', 'Cache folder'),
      el('p', 'muted file-path', (snapshot.workspace ? snapshot.cacheFolder : snapshot.folder) || 'No cache folder selected.'),
      el('div', 'toolbar', detect, choose)),
    el('section', 'settings-panel panel',
      el('h2', '', 'Roblox downloads'),
      form, el('div', 'toolbar',
        ...(keySource ? [detectKey, clear] : [el('span', 'badge', 'No API key loaded'), detectKey]))));
}

function render() {
  closeInlinePanel?.(false);
  disposePreview?.();
  disposePreview = undefined;
  updateLibrarySelection = undefined;
  refreshLibraryResults = undefined;
  observer?.disconnect();
  thumbQueue.length = 0;
  thumbItems = new WeakMap();
  content.replaceChildren();
  search.hidden = view !== 'library';
  if (search.hidden) assetSearch.close();
  location.textContent = snapshot.workspace ? 'Editing workspace' : snapshot.folder ? snapshot.folder.split(/[\\/]/).filter(Boolean).pop() || snapshot.folder : 'No folder';
  locationBar.title = snapshot.folder || 'No folder selected';
  for (const [id, item] of navButtons) {
    if (id === view) item.setAttribute('aria-current', 'page');
    else item.removeAttribute('aria-current');
  }
  if (view === 'library') renderLibrary();
  else if (view === 'changes') renderChanges();
  else renderSettings();
  syncBusy();
}

function sameAsset(before: Asset, after: Asset) {
  return before.id === after.id && before.checksum === after.checksum && before.name === after.name &&
    before.changed === after.changed && before.defaultName === after.defaultName &&
    before.customName === after.customName && before.sourceAssetId === after.sourceAssetId &&
    before.modified === after.modified && before.size === after.size && before.category === after.category &&
    before.kind === after.kind && before.previewOnly === after.previewOnly && before.mime === after.mime &&
    before.databaseEntry === after.databaseEntry;
}

async function refreshLibrary() {
  if (busy || refreshing || view !== 'library' || filterMenuOpen()) return;
  refreshing = true;
  const job = sequence;
  try {
    const next = await invoke<Snapshot>('refresh_cache');
    if (busy || job !== sequence || view !== 'library' || filterMenuOpen()) return;
    const previous = snapshot;
    const sameFolder = previous.folder === next.folder && previous.canClearCache === next.canClearCache &&
      previous.detected === next.detected && previous.workspace === next.workspace;
    if (sameFolder && previous.assets.length === next.assets.length &&
        previous.assets.every((asset, index) => sameAsset(asset, next.assets[index]))) return;
    const priorSelection = previous.assets.find(asset => asset.id === selectedId);
    const nextSelection = next.assets.find(asset => asset.id === selectedId);
    selectedId = nextSelection?.id ?? null;
    snapshot = next;
    if (sameFolder && refreshLibraryResults) {
      refreshLibraryResults(true);
      if (priorSelection && (!nextSelection || !sameAsset(priorSelection, nextSelection))) updateLibrarySelection?.();
      syncBusy();
    } else render();
    if (previous.folder !== next.folder || previous.assets.length !== next.assets.length) {
      announce(scanMessage());
    }
  } catch {
    // Leave the last known library visible. The explicit scan reports errors.
  } finally {
    refreshing = false;
  }
}

function filterMenuOpen() {
  return content.querySelector('.asset-filter-trigger[aria-expanded="true"]') !== null;
}

render();
const scanListener = listen<ScanUpdate>('cache-scan-progress', ({ payload }) => showScanProgress(payload))
  .catch(() => {});
void listen<{ job: number; message: string }>('asset-progress', ({ payload }) => {
  if (busy && payload.job === sequence) announce(payload.message);
}).catch(() => {
  // Operations still report their result if the native progress event is unavailable.
});
if (native) {
  void scanListener.then(() => operate('Finding and scanning the cache…', async job => {
    showScanProgress({ job, checked: 0, total: null });
    [snapshot] = await Promise.all([
      invoke<Snapshot>('scan', { job }),
      invoke<KeySource | null>('api_key_status').then(source => {
        keySource = source;
        if (view === 'settings') render();
      }).catch(() => {}),
    ]);
    return scanMessage();
  }));
  window.addEventListener('focus', () => { void refreshLibrary(); });
  window.setInterval(() => { void refreshLibrary(); }, refreshInterval);
}
