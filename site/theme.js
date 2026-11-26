const systemDark = window.matchMedia?.('(prefers-color-scheme: dark)');

function applySystemTheme({ matches }) {
  if (matches) {
    document.documentElement.dataset.theme = 'dark';
  } else {
    delete document.documentElement.dataset.theme;
  }
}

if (systemDark) {
  applySystemTheme(systemDark);
  systemDark.addEventListener?.('change', applySystemTheme);
}
