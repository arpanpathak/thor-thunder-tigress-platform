// Formulas, diagrams and drawings in replies. markdown.js only marks where they
// are; this module draws them. The libraries are large, so each one loads the
// first time a reply needs it, and the browser keeps it after that.
import { element } from "./dom.js";

const KATEX_SCRIPT = "katex-0.19.0.min.js";
const KATEX_STYLE = "katex-0.19.0.css";
const MERMAID_SCRIPT = "mermaid-12.1.0.min.js";

// A drawing larger than this is shown as code: it is a model rambling, not art.
const MAX_SVG = 512 * 1024;

// ---- loading ----
const loading = new Map();

function script(src) {
  if (!loading.has(src)) {
    loading.set(src, new Promise((resolve, reject) => {
      const node = element("script");
      node.src = src;
      node.onload = resolve;
      node.onerror = () => { loading.delete(src); node.remove(); reject(new Error(`${src} did not load`)); };
      document.head.append(node);
    }));
  }

  return loading.get(src);
}

function stylesheet(href) {
  if (document.querySelector(`link[href="${href}"]`)) return;

  const link = element("link");
  link.rel = "stylesheet";
  link.href = href;
  document.head.append(link);
}

// ---- formulas ----
// `trust: false` keeps \href, \url, \includegraphics and the \html… commands
// out, so a formula can only ever draw math. The limits stop a formula from
// growing without bound or expanding macros forever.
const KATEX_OPTIONS = { throwOnError: false, trust: false, strict: "ignore", maxSize: 30, maxExpand: 1000, output: "htmlAndMathml" };

function typesetOne(node) {
  try {
    globalThis.katex.render(node.dataset.tex, node, { ...KATEX_OPTIONS, displayMode: node.classList.contains("display") });
    node.classList.add("typeset");
  } catch {
    // The TeX source stays on the page as text.
  }
}

/** Draws every formula under `root`: at once when KaTeX is loaded, else after it loads. */
export function typeset(root) {
  const pending = () => root.querySelectorAll(".math:not(.typeset)");
  if (!pending().length) return;

  if (globalThis.katex) { pending().forEach(typesetOne); return; }

  stylesheet(KATEX_STYLE);
  script(KATEX_SCRIPT).then(() => pending().forEach(typesetOne)).catch(() => {});
}

// ---- drawings and diagrams ----
// Both end up as an <img> of an SVG data URL. An image is its own sandbox: no
// script in it runs and nothing it links to loads, so a model can draw anything
// without being able to reach the page.
function svgImage(svg, label) {
  const text = /\sxmlns=/.test(svg.slice(0, svg.indexOf(">"))) ? svg : svg.replace(/<svg\b/, '<svg xmlns="http://www.w3.org/2000/svg"');
  const image = element("img");
  image.alt = label;
  image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(text)}`;

  return image;
}

// A theme colour as #rrggbb. Themes may write a colour as color-mix(…), which
// Mermaid can't read: the browser resolves it on a hidden element, and a
// one-pixel canvas turns whatever it resolved to into plain red, green and blue.
let pixel;

function solid(value) {
  const probe = element("span");
  probe.style.color = value;
  probe.hidden = true;
  document.body.append(probe);
  const resolved = getComputedStyle(probe).color;
  probe.remove();

  pixel ??= Object.assign(element("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
  pixel.clearRect(0, 0, 1, 1);
  pixel.fillStyle = "#808080";
  pixel.fillStyle = resolved;
  pixel.fillRect(0, 0, 1, 1);

  return `#${[...pixel.getImageData(0, 0, 1, 1).data.slice(0, 3)].map((n) => n.toString(16).padStart(2, "0")).join("")}`;
}

// The page theme's colours, so a diagram matches the theme it is drawn in.
function themeColours() {
  const style = getComputedStyle(document.documentElement);
  const colour = (name) => solid(style.getPropertyValue(name).trim());
  const background = colour("--bg");
  const [r, g, b] = background.match(/[\da-f]{2}/gi).map((hex) => parseInt(hex, 16));

  return {
    darkMode: 0.2126 * r + 0.7152 * g + 0.0722 * b < 128,
    background,
    primaryColor: colour("--panel"),
    primaryTextColor: colour("--text"),
    primaryBorderColor: colour("--accent"),
    secondaryColor: colour("--user"),
    tertiaryColor: colour("--bar"),
    lineColor: colour("--muted"),
    textColor: colour("--text"),
    noteBkgColor: colour("--user"),
    noteTextColor: colour("--text"),
    fontFamily: getComputedStyle(document.body).fontFamily,
  };
}

// Mermaid keeps global state while it draws, so diagrams are drawn one at a time.
// A drawing is kept by its source and theme: a reply re-renders on every token
// while it streams, and the same diagram must not be drawn again each time.
const drawn = new Map();
const finished = new Map();
let queue = Promise.resolve();
let counter = 0;

function drawMermaid(source) {
  const colours = themeColours();
  const key = `${JSON.stringify(colours)}\n${source}`;

  if (!drawn.has(key)) {
    const result = queue.then(async () => {
      await script(MERMAID_SCRIPT);
      const { mermaid } = globalThis;
      mermaid.initialize({
        startOnLoad: false, securityLevel: "strict", htmlLabels: false, suppressErrorRendering: true,
        theme: "base", themeVariables: colours, fontFamily: colours.fontFamily,
      });
      const { svg } = await mermaid.render(`thor-diagram-${counter += 1}`, source);
      finished.set(key, svg);

      return svg;
    });

    queue = result.catch(() => {});
    drawn.set(key, result);
  }

  return drawn.get(key);
}

function finishedMermaid(source) {
  return finished.get(`${JSON.stringify(themeColours())}\n${source}`);
}

function showDrawing(figure, image) {
  figure.querySelector(".view").replaceChildren(image);
  figure.classList.add("drawn");
  figure.dataset.svg = decodeURIComponent(image.src.slice(image.src.indexOf(",") + 1));
}

function showFailure(figure, reason) {
  figure.classList.add("failed");
  figure.querySelector(".view").replaceChildren(element("div", "figure-note", reason));
}

/** Draws every finished ```mermaid and ```svg block under `root`. */
export function drawFigures(root) {
  root.querySelectorAll(".figure:not(.drawn):not(.failed):not(.drawing)").forEach((figure) => {
    const source = figure.querySelector("pre").textContent;

    if (figure.dataset.kind === "svg") {
      if (source.length > MAX_SVG || !/<svg\b/i.test(source)) { showFailure(figure, "Not a drawing this page can show; the code is below."); return; }

      const image = svgImage(source.trim(), "Drawing from the reply");
      image.addEventListener("error", () => showFailure(figure, "This SVG has an error, so it can't be drawn; the code is below."), { once: true });
      showDrawing(figure, image);

      return;
    }

    const ready = finishedMermaid(source);
    if (ready) { showDrawing(figure, svgImage(ready, "Diagram from the reply")); return; }

    figure.classList.add("drawing");
    figure.querySelector(".view").replaceChildren(element("div", "figure-note", "drawing the diagram…"));

    drawMermaid(source)
      .then((svg) => showDrawing(figure, svgImage(svg, "Diagram from the reply")))
      .catch((error) => showFailure(figure, `Couldn't draw this diagram: ${String(error.message || error).split("\n")[0]}`))
      .finally(() => figure.classList.remove("drawing"));
  });
}

/** Draws everything rich in a rendered reply. */
export function enrich(root) {
  typeset(root);
  drawFigures(root);
}
