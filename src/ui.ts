export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "default" | "small" | "icon";

interface ButtonOptions {
  variant?: ButtonVariant;
  icon?: string;
  type?: "button" | "submit";
  ariaLabel?: string;
  size?: ButtonSize;
}

// Outline segments share a 24px grid with round caps and joins.
const outlines = new Map<string, string[]>(Object.entries({
  folder: ["M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"],
  "folder-open": ["M3 17V7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2", "m3 17 2-6h17l-2.2 7a2 2 0 0 1-1.9 1.4H5A2 2 0 0 1 3 17Z"],
  image: ["M5 4h14a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1Z", "M9 9h.01", "m4 17 4.5-4.5 3 3 3.5-4 5 5.5"],
  film: ["M5 4h14a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1Z", "M8 4v16m8-16v16M4 9h4m8 0h4M4 15h4m8 0h4"],
  search: ["M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14Zm5-2 4 4"],
  scan: ["M8 4H5a1 1 0 0 0-1 1v3m12-4h3a1 1 0 0 1 1 1v3M4 16v3a1 1 0 0 0 1 1h3m8 0h3a1 1 0 0 0 1-1v-3M7 12h10"],
  settings: ["M6 8h12M6 16h12", "M9 5.5v5m6 3v5"],
  filter: ["M4 7h16M7 12h10m-7 5h4"],
  person: ["M16 8a4 4 0 1 1-8 0 4 4 0 0 1 8 0Zm-11 12v-1a4 4 0 0 1 4-4h6a4 4 0 0 1 4 4v1"],
  music: ["M9 18V5l11-2v13M9 18a3 2 0 1 1-6 0 3 2 0 0 1 6 0Zm11-2a3 2 0 1 1-6 0 3 2 0 0 1 6 0Z"],
  history: ["M4 11a8 8 0 1 1 2.5 6.8M4 6v5h5m3-3v4l2.5 1.5"],
  "rotate-ccw": ["M4 10a8 8 0 1 1 1.8 8.2M4 5v5h5"],
  upload: ["M12 16V4m-4 4 4-4 4 4M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3"],
  download: ["M12 4v12m-4-4 4 4 4-4M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3"],
  "chevron-right": ["m9 5 7 7-7 7"],
  "chevron-down": ["m5 9 7 7 7-7"],
  "arrow-left": ["m10 6-6 6 6 6M4 12h16"],
  check: ["m5 12 4 4L19 6"],
  alert: ["M10.5 5a1.7 1.7 0 0 1 3 0l7.3 12.4a1.7 1.7 0 0 1-1.5 2.6H4.7a1.7 1.7 0 0 1-1.5-2.6Z", "M12 9v4m0 3h.01"],
  x: ["m6 6 12 12M18 6 6 18"],
  trash: ["M3 6h18M8 6V4h8v2m3 0-1 14H6L5 6m5 4v6m4-6v6"],
  grid: ["M4 4h6v6H4Zm10 0h6v6h-6ZM4 14h6v6H4Zm10 0h6v6h-6Z"],
  list: ["M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01"],
  play: ["m8 5 12 7-12 7Z"],
  pause: ["M9 5v14m6-14v14"],
  "volume-2": ["M12 5 7 9H4v6h3l5 4V5Z", "M15 9a4.2 4.2 0 0 1 0 6m2-8a7 7 0 0 1 0 10"],
  "volume-x": ["M12 5 7 9H4v6h3l5 4V5Z", "m16 10 4 4m0-4-4 4"],
  loader: ["M12 3a9 9 0 1 1-9 9"],
  "hard-drive": ["m4 12 2.7-6h10.6l2.7 6m-15 0h14a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1Z", "M7.5 15.5h.01m3.5 0h.01"],
  file: ["M7 3h7l4 4v13a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1Zm7 0v5h4"],
  edit: ["m14 6 4 4M4 20l1-5L16.3 3.7a1 1 0 0 1 1.4 0l2.6 2.6a1 1 0 0 1 0 1.4L9 19Z"],
  key: ["M14.5 4a5.5 5.5 0 0 0-5.03 7.73L4 17.2V20h2.8l1.3-1.3v-2.1h2.1l2-2A5.5 5.5 0 1 0 14.5 4Z", "M16 8h.01"],
  inbox: ["m4 13 3-7h10l3 7v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1Zm0 0h5l1.5 2h3l1.5-2h5"],
  box: ["m12 3 9 5v8l-9 5-9-5V8l9-5Zm0 10 9-5M3 8l9 5v8"],
  "image-off": ["M8 4h11a1 1 0 0 1 1 1v11m-1 4H5a1 1 0 0 1-1-1V5m0 12 4.5-4.5L12 16m2-3 1-1 5 5M3 3l18 18"],
  info: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Zm0-10v5m0-8h.01"],
  "external-link": ["M14 4h6v6m0-6-9 9m7-1v6a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"],
  "check-circle": ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Zm-4-9 2.5 2.5L16 9"],
  clock: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Zm0-13v4l3 2"],
  refresh: ["M20 9a8 8 0 0 0-13.3-3L4 9m0-5v5h5m-5 6a8 8 0 0 0 13.3 3l2.7-3m0 5v-5h-5"],
  eye: ["M3 12s3-6 9-6 9 6 9 6-3 6-9 6-9-6-9-6Zm9 2.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5Z"],
  "eye-off": ["M3 3l18 18m-3-4a10 10 0 0 1-6 1c-6 0-9-6-9-6a15 15 0 0 1 4-4m3-1.7a9 9 0 0 1 2-.3c6 0 9 6 9 6a13 13 0 0 1-1.4 2"],
  layers: ["m12 3 9 5-9 5-9-5Zm-9 9 9 5 9-5m-18 4 9 5 9-5"],
}));

const svgNamespace = "http://www.w3.org/2000/svg";

export function icon(name: string): SVGSVGElement {
  const element = document.createElementNS(svgNamespace, "svg");
  element.setAttribute("viewBox", "0 0 24 24");
  element.setAttribute("fill", "none");
  element.setAttribute("stroke", "currentColor");
  element.setAttribute("stroke-width", "1.7");
  element.setAttribute("stroke-linecap", "round");
  element.setAttribute("stroke-linejoin", "round");
  element.setAttribute("aria-hidden", "true");
  element.setAttribute("focusable", "false");
  element.classList.add("icon", `icon-${name}`);

  for (const outline of outlines.get(name) ?? outlines.get("file")!) {
    const path = document.createElementNS(svgNamespace, "path");
    path.setAttribute("d", outline);
    element.append(path);
  }

  return element;
}

export function button(label: string, options: ButtonOptions = {}): HTMLButtonElement {
  const { variant = "secondary", size = "default", type = "button", ariaLabel } = options;
  const element = document.createElement("button");
  element.type = type;
  element.classList.add("button", `button--${variant}`, `button--${size}`);

  if (ariaLabel || (size === "icon" && label)) {
    element.setAttribute("aria-label", ariaLabel || label);
  }
  if (options.icon) element.append(icon(options.icon));
  if (label && size !== "icon") {
    const text = document.createElement("span");
    text.className = "button-label";
    text.textContent = label;
    element.append(text);
  }

  return element;
}
