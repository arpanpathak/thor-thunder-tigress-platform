export const $ = (id) => document.getElementById(id);

// ---- DOM helpers ----
export function element(tag, className, text) {
  const node = document.createElement(tag);

  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;

  return node;
}

const SVG = "http://www.w3.org/2000/svg";

export function cub(viewBox = "0 0 440 460") {
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", viewBox);
  svg.setAttribute("aria-hidden", "true");

  const use = document.createElementNS(SVG, "use");
  use.setAttribute("href", "#cub-art");
  svg.append(use);

  return svg;
}
