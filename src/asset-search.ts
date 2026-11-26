import { assetCategories, type AssetCategory } from './asset-filters';
import { button, icon } from './ui';

interface SearchAsset {
  id: string;
  name: string;
  path: string;
  category: AssetCategory;
}

const limit = 8;

export function createAssetSearch(onQuery: () => void, onSelect: (id: string) => void) {
  const element = document.createElement('div');
  element.className = 'search asset-search';
  const input = document.createElement('input');
  input.type = 'search';
  input.placeholder = 'Search names, IDs, and types';
  input.setAttribute('role', 'combobox');
  input.setAttribute('aria-label', 'Search cached assets');
  input.setAttribute('aria-autocomplete', 'list');
  input.setAttribute('aria-expanded', 'false');
  input.setAttribute('autocomplete', 'off');

  const toggle = button('', { icon: 'chevron-down', size: 'icon', variant: 'ghost', ariaLabel: 'Toggle search results' });
  toggle.classList.add('asset-search-toggle');
  toggle.setAttribute('aria-expanded', 'false');
  toggle.tabIndex = -1;
  toggle.disabled = true;
  const popup = document.createElement('div');
  popup.className = 'asset-search-popup panel';
  popup.inert = true;
  const list = document.createElement('div');
  list.id = 'asset-search-results';
  list.setAttribute('role', 'listbox');
  list.setAttribute('aria-label', 'Matching assets');
  input.setAttribute('aria-controls', list.id);
  const empty = document.createElement('p');
  empty.className = 'asset-search-empty muted';
  empty.setAttribute('role', 'status');
  empty.textContent = 'No matching assets';
  popup.append(list, empty);
  element.append(icon('search'), input, toggle, popup);

  let matches: SearchAsset[] = [];
  let active = -1;

  function highlight(next: number) {
    const options = Array.from(list.children);
    options[active]?.setAttribute('aria-selected', 'false');
    active = next;
    const option = options[active];
    if (option) {
      option.setAttribute('aria-selected', 'true');
      input.setAttribute('aria-activedescendant', option.id);
      option.scrollIntoView({ block: 'nearest' });
    } else {
      input.removeAttribute('aria-activedescendant');
    }
  }

  function close() {
    popup.inert = true;
    input.setAttribute('aria-expanded', 'false');
    toggle.setAttribute('aria-expanded', 'false');
    highlight(-1);
  }

  function choose(asset: SearchAsset) {
    close();
    onSelect(asset.id);
  }

  function renderMatches() {
    highlight(-1);
    const options = matches.map((asset, index) => {
      const option = document.createElement('div');
      option.id = list.id + '-' + index;
      option.className = 'asset-search-option';
      option.setAttribute('role', 'option');
      option.setAttribute('aria-selected', 'false');
      const title = document.createElement('strong');
      title.className = 'truncate';
      title.textContent = asset.name || asset.path;
      const category = assetCategories[asset.category];
      const type = document.createElement('span');
      type.className = 'asset-search-type truncate';
      type.textContent = category.singular;
      option.append(icon(category.icon), title, type);
      option.addEventListener('click', () => choose(asset));
      return option;
    });
    list.replaceChildren(...options);
    empty.hidden = !!options.length;
  }

  function open() {
    if (!popup.inert || (!matches.length && !input.value.trim())) return;
    renderMatches();
    popup.inert = false;
    input.setAttribute('aria-expanded', 'true');
    toggle.setAttribute('aria-expanded', 'true');
  }

  input.addEventListener('focus', open);
  input.addEventListener('click', open);
  input.addEventListener('blur', close);
  input.addEventListener('input', () => { onQuery(); open(); });
  input.addEventListener('keydown', event => {
    if (event.isComposing) return;
    if (event.key === 'Escape' && !popup.inert) {
      event.preventDefault();
      close();
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      open();
      if (!matches.length) return;
      event.preventDefault();
      const next = event.key === 'ArrowDown' ? (active + 1) % matches.length
        : active <= 0 ? matches.length - 1 : active - 1;
      highlight(next);
    } else if (event.key === 'Enter' && !popup.inert && matches.length) {
      event.preventDefault();
      choose(matches[Math.max(0, active)]);
    }
  });
  toggle.addEventListener('click', () => {
    if (!popup.inert) close();
    else { input.focus(); open(); }
  });
  element.addEventListener('mousedown', event => {
    if (event.button === 0 && event.target instanceof Element
        && event.target.closest('.asset-search-option, .asset-search-toggle')) event.preventDefault();
  });
  document.addEventListener('pointerdown', event => {
    if (event.target instanceof Node && !element.contains(event.target)) close();
  });

  return {
    element,
    input,
    close,
    setMatches(assets: SearchAsset[]) {
      const next = assets.slice(0, limit);
      toggle.disabled = !next.length && !input.value.trim();
      const unchanged = next.length === matches.length && next.every((asset, index) => {
        const previous = matches[index];
        return asset.id === previous.id && asset.name === previous.name
          && asset.path === previous.path && asset.category === previous.category;
      });
      matches = next;
      if (!unchanged && !popup.inert) renderMatches();
    },
  };
}
