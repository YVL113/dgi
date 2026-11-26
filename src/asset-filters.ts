import { button, icon } from './ui';

export const assetCategories = {
  image: { label: 'Images & decals', singular: 'Image / decal', icon: 'image' },
  texture: { label: 'Textures', singular: 'Texture', icon: 'layers' },
  animation: { label: 'Animations', singular: 'Animation', icon: 'film' },
  avatar: { label: 'Avatars & clothing', singular: 'Avatar / clothing', icon: 'person' },
  model: { label: 'Models', singular: 'Model', icon: 'box' },
  mesh: { label: 'Meshes', singular: 'Mesh', icon: 'box' },
  audio: { label: 'Audio', singular: 'Audio', icon: 'music' },
} as const;

export type AssetCategory = keyof typeof assetCategories;

interface FilterableAsset {
  category: AssetCategory;
  name: string;
  defaultName?: string;
  sourceAssetId?: string;
  path: string;
}

const categories = Object.keys(assetCategories) as AssetCategory[];
const preference = 'dgi.asset-filters.v1';
const hidden = new Set<AssetCategory>();
let nextMenu = 0;

try {
  const saved: unknown = JSON.parse(localStorage.getItem(preference) ?? '[]');
  if (Array.isArray(saved)) {
    for (const category of categories) {
      if (saved.includes(category)) hidden.add(category);
    }
  }
} catch {
  // Filtering remains available if the WebView cannot read preferences.
}

export function noCategoriesSelected() {
  return hidden.size === categories.length;
}

export function filterAssets<T extends FilterableAsset>(assets: T[], rawQuery: string): T[] {
  const terms = rawQuery.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return assets.filter(asset => {
    if (hidden.has(asset.category)) return false;
    if (!terms.length) return true;
    const category = assetCategories[asset.category];
    const fields = [asset.name, asset.defaultName ?? '', asset.sourceAssetId ?? '', asset.path, category.label, category.singular]
      .map(value => value.toLocaleLowerCase());
    return terms.every(term => fields.some(value => value.includes(term)));
  });
}

function closeFilter(root: HTMLElement, restoreFocus = false) {
  const trigger = root.querySelector<HTMLButtonElement>('.asset-filter-trigger');
  const panel = root.querySelector<HTMLElement>('.asset-filter-panel');
  if (panel) panel.hidden = true;
  trigger?.setAttribute('aria-expanded', 'false');
  if (restoreFocus) trigger?.focus();
}

document.addEventListener('pointerdown', event => {
  if (!(event.target instanceof Node)) return;
  for (const filter of document.querySelectorAll<HTMLElement>('.asset-filters')) {
    if (!filter.contains(event.target)) closeFilter(filter);
  }
});

export function createAssetFilters(assets: FilterableAsset[], onChange: () => void): HTMLElement {
  const counts = new Map<AssetCategory, number>();
  for (const { category } of assets) counts.set(category, (counts.get(category) ?? 0) + 1);
  const root = document.createElement('div');
  root.className = 'asset-filters';
  const trigger = button('Filters', { icon: 'filter', variant: 'secondary', size: 'small' });
  trigger.classList.add('asset-filter-trigger');
  const hiddenCount = document.createElement('span');
  hiddenCount.className = 'filter-hidden-count';
  trigger.append(hiddenCount);
  const panel = document.createElement('div');
  panel.id = 'asset-filters-' + ++nextMenu;
  panel.className = 'asset-filter-panel panel';
  panel.hidden = true;
  trigger.setAttribute('aria-controls', panel.id);
  trigger.setAttribute('aria-expanded', 'false');
  trigger.addEventListener('click', () => {
    panel.hidden = !panel.hidden;
    trigger.setAttribute('aria-expanded', String(!panel.hidden));
  });
  root.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || panel.hidden) return;
    event.preventDefault();
    event.stopPropagation();
    closeFilter(root, true);
  });

  const options = document.createElement('fieldset');
  const legend = document.createElement('legend');
  legend.textContent = 'Asset types';
  options.append(legend);
  const items = new Map<AssetCategory, HTMLInputElement>();
  for (const category of categories) {
    const option = document.createElement('label');
    option.className = 'asset-filter-option';
    const input = document.createElement('input');
    input.type = 'checkbox';
    const name = document.createElement('span');
    name.textContent = assetCategories[category].label;
    const count = document.createElement('span');
    count.className = 'asset-filter-count';
    count.textContent = (counts.get(category) ?? 0).toLocaleString();
    count.setAttribute('aria-hidden', 'true');
    option.append(input, icon(assetCategories[category].icon), name, count);
    options.append(option);
    items.set(category, input);
    input.addEventListener('change', () => {
      if (input.checked) hidden.delete(category);
      else hidden.add(category);
      changed();
    });
  }

  const actions = document.createElement('div');
  actions.className = 'asset-filter-actions';
  const clear = button('Clear all', { variant: 'ghost', size: 'small' });
  clear.addEventListener('click', () => {
    for (const category of categories) hidden.add(category);
    changed();
  });
  const reset = button('Show all', { variant: 'secondary', size: 'small' });
  reset.addEventListener('click', () => {
    hidden.clear();
    changed();
  });
  actions.append(clear, reset);

  function update() {
    for (const [category, input] of items) input.checked = !hidden.has(category);
    hiddenCount.textContent = String(hidden.size);
    hiddenCount.hidden = !hidden.size;
    trigger.setAttribute('aria-label', hidden.size ? 'Filters, ' + hidden.size + ' types hidden' : 'Filters');
  }

  function changed() {
    try { localStorage.setItem(preference, JSON.stringify([...hidden])); } catch {
      // The current filter still applies when local storage is unavailable.
    }
    update();
    onChange();
  }

  update();
  panel.append(options, actions);
  root.append(trigger, panel);
  return root;
}
