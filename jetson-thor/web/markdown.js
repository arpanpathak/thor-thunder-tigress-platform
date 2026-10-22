import { element } from "./dom.js";

// ---- links: the one place an address becomes an anchor ----
const EXTERNAL = /^https?:\/\//i;

function anchor(label, href) {
  const node = element("a", "", label);
  node.href = href;
  node.target = "_blank";
  node.rel = "noreferrer";

  return node;
}

// A model answer may name an address, and a job answer usually does. The
// anchor is built here, never by handing text to the HTML parser, and only
// http and https are ever given an href.
function externalLink(label, href) {
  return EXTERNAL.test(href) ? anchor(label, href) : element("span", "", label);
}

// ---- syntax highlighting: a small lexer, not a parser ----
const KEYWORDS = {
  rust: "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while",
  python: "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return self True try while with yield",
  go: "break case chan const continue default defer else fallthrough for func go goto if import interface map nil package range return select struct switch type var true false",
  c: "auto bool break case char class const constexpr continue default delete do double else enum explicit extern false float for if inline int long namespace new nullptr private protected public return short signed sizeof static struct switch template this true typedef typename union unsigned using virtual void volatile while",
  js: "async await break case catch class const continue default delete do else export extends false finally for function if import in instanceof let new null of return static super switch this throw true try typeof undefined var void while yield",
  shell: "if then else elif fi for in do done while case esac function return export local set echo cd sudo",
  data: "true false null",
};
const ALIASES = { rs: "rust", rust: "rust", py: "python", python: "python", go: "go", c: "c", h: "c", cpp: "c", "c++": "c",
  cc: "c", cu: "c", cuda: "c", js: "js", javascript: "js", ts: "js", typescript: "js", sh: "shell", bash: "shell",
  shell: "shell", zsh: "shell", console: "shell", toml: "data", json: "data", yaml: "data", yml: "data" };

const KINDS = ["comment", "string", "attr", "lifetime", "number", "macro", "word"];
const NEVER = String.raw`(?!)`;

function lexer(language) {
  const hash = ["python", "shell", "data"].includes(language);
  const strings = {
    python: String.raw`"""[\s\S]*?"""|'''[\s\S]*?'''|[rbf]?"(?:\\.|[^"\\\n])*"|[rbf]?'(?:\\.|[^'\\\n])*'`,
    rust: String.raw`b?r(?<hashes>#*)"[\s\S]*?"\k<hashes>|b?"(?:\\.|[^"\\])*"|b?'(?:\\.|[^'\\\n])'`,
  };

  const patterns = {
    comment: hash ? String.raw`#.*` : String.raw`\/\/.*|\/\*[\s\S]*?\*\/`,
    string: strings[language] || String.raw`"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'|` + "`[^`]*`",
    attr: language === "rust" ? String.raw`#!?\[[^\]\n]*\]` : language === "python" ? String.raw`@\w+` : NEVER,
    lifetime: language === "rust" ? String.raw`'[a-z_]\w*\b(?!')` : NEVER,
    number: String.raw`\b(?:0x[\da-fA-F_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:e[+-]?\d+)?)(?:[iu](?:8|16|32|64|128|size)|f32|f64)?\b`,
    macro: language === "rust" ? String.raw`\b[a-z_]\w*!` : NEVER,
    word: String.raw`\b[A-Za-z_]\w*\b`,
  };

  return new RegExp(KINDS.map((kind) => `(?<${kind}>${patterns[kind]})`).join("|"), "g");
}

function highlight(code, lang) {
  const language = ALIASES[lang] || (lang ? "" : "rust");
  const fragment = document.createDocumentFragment();

  if (!language) { fragment.append(code); return fragment; }

  const keywords = new Set(KEYWORDS[language].split(" "));
  let position = 0;

  for (const match of code.matchAll(lexer(language))) {
    const word = match[0];
    const kind = KINDS.find((name) => match.groups[name] !== undefined);
    const style = kind !== "word" ? kind
      : keywords.has(word) ? "keyword"
      : /^[A-Z]/.test(word) ? "type"
      : code[match.index + word.length] === "(" ? "call" : "";
    if (!style) continue;

    fragment.append(code.slice(position, match.index), element("span", `tok-${style}`, word));
    position = match.index + word.length;
  }

  fragment.append(code.slice(position));

  return fragment;
}

// ---- inline markdown: code, math, emphasis and links, all built from DOM nodes ----
// The pattern matches, in order, a code span, a formula, bold, italic, a markdown link,
// an <autolink>, a bare address, and a <br> the model writes to break a line
// inside a table cell. A bare address matters most in job answers, where the
// model writes the posting's URL as plain text.
//
// A formula is $$…$$, \[…\], \(…\) or $…$. A single $ follows Pandoc's rule so prices
// stay text: the opening $ has no space after it, the closing $ has no space
// before it and no digit after it, so "$5 and $10" is not math.
const INLINE = /`(?<code>[^`\n]+)`|\$\$(?<display>[^$\n]+?)\$\$|\\\[(?<bracket>[^\n]+?)\\\]|\\\((?<paren>[^\n]+?)\\\)|(?<![\\$\w])\$(?<math>[^\s$](?:[^$\n]*?[^\s$\\])?)\$(?![\d$])|\*\*(?<bold>[^*\n]+)\*\*|\*(?<italic>[^*\n]+)\*|(?<!\w)_(?<under>[^_\n]+)_(?!\w)|\[(?<text>[^\]\n]+)\]\((?<href>https?:\/\/[^)\s]+)\)|<(?<auto>https?:\/\/[^>\s]+)>|(?<bare>(?:https?:\/\/|www\.)[^\s<>"'`]+)|(?<br><\/?[bB][rR]\s*\/?>)/g;

// Whether a run of text has more closing than opening parentheses, so a URL
// that ends inside `(see …)` loses the sentence's bracket and not its own.
function unbalancedClose(text) {
  let depth = 0;

  for (const character of text) {
    if (character === "(") depth += 1;
    else if (character === ")") depth -= 1;
  }

  return depth < 0;
}

// The address and the sentence punctuation written after it, split apart.
function splitTrailing(raw) {
  let url = raw;
  let tail = "";

  while (url && ".,;:!?…".includes(url[url.length - 1])) {
    tail = url[url.length - 1] + tail;
    url = url.slice(0, -1);
  }

  while (url.endsWith(")") && unbalancedClose(url)) {
    tail = `)${tail}`;
    url = url.slice(0, -1);
  }

  return { url, tail };
}

function bareLink(raw) {
  const { url, tail } = splitTrailing(raw);
  if (!url) return { node: document.createTextNode(raw), tail: "" };

  const href = url.startsWith("www.") ? `https://${url}` : url;

  return { node: externalLink(url, href), tail };
}

function emphasis(tag, text, insideLink = false) {
  const node = element(tag);
  inline(node, text, insideLink);

  return node;
}

// `insideLink` stops a bare address in a link's own label from becoming an
// anchor inside an anchor, which is not valid HTML.
function inline(parent, text, insideLink = false) {
  let position = 0;
  for (const match of text.matchAll(INLINE)) {
    parent.append(document.createTextNode(text.slice(position, match.index)));
    const groups = match.groups;
    if (groups.code !== undefined) parent.append(element("code", "", groups.code));
    else if (groups.display !== undefined) parent.append(formula(groups.display, true, "span"));
    else if (groups.bracket !== undefined) parent.append(formula(groups.bracket, true, "span"));
    else if (groups.paren !== undefined) parent.append(formula(groups.paren, false, "span"));
    else if (groups.math !== undefined) parent.append(formula(groups.math, false, "span"));
    else if (groups.bold !== undefined) parent.append(emphasis("strong", groups.bold, insideLink));
    else if (groups.italic !== undefined) parent.append(emphasis("em", groups.italic, insideLink));
    else if (groups.under !== undefined) parent.append(emphasis("em", groups.under, insideLink));
    else if (groups.br !== undefined) parent.append(element("br"));
    else if (groups.text !== undefined) {
      const link = externalLink("", groups.href);
      inline(link, groups.text, true);
      parent.append(link);
    } else if (insideLink) {
      parent.append(document.createTextNode(match[0]));
    } else {
      const { node, tail } = bareLink(groups.auto ?? groups.bare);
      parent.append(node, tail);
    }
    position = match.index + match[0].length;
  }

  parent.append(document.createTextNode(text.slice(position)));
}

// A formula's TeX, marked for rich.js to typeset; until then it reads as TeX.
function formula(tex, display, tag) {
  const node = element(tag, display ? "math display" : "math", tex);
  node.dataset.tex = tex.trim();

  return node;
}

function copyButton(code) {
  const copy = element("button", "", "copy");
  copy.type = "button";

  copy.addEventListener("click", async () => {
    try { await navigator.clipboard.writeText(code); copy.textContent = "copied"; }
    catch { copy.textContent = "select + copy"; }

    setTimeout(() => { copy.textContent = "copy"; }, 1400);
  });

  return copy;
}

function codeBlock(code, lang) {
  const block = element("div", "code");
  const bar = element("div", "bar");

  bar.append(element("span", "", lang || "text"), copyButton(code));

  const pre = element("pre");
  const node = element("code");
  node.append(highlight(code, lang));
  pre.append(node);

  block.append(bar, pre);

  return block;
}

// A ```mermaid diagram or ```svg drawing. rich.js draws it into `.view`; the
// code stays one click away, and the drawing can be saved as an .svg file.
function figureBlock(code, kind, lang) {
  const block = element("div", "code figure");
  block.dataset.kind = kind;

  const bar = element("div", "bar");
  const toggle = element("button", "", "code");
  toggle.type = "button";
  toggle.addEventListener("click", () => {
    const showing = block.classList.toggle("show-code");
    toggle.textContent = showing ? (kind === "svg" ? "drawing" : "diagram") : "code";
  });

  const save = element("button", "", "download");
  save.type = "button";
  save.addEventListener("click", () => {
    const svg = block.dataset.svg || code;
    const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
    const link = element("a");
    link.href = url;
    link.download = kind === "svg" ? "drawing.svg" : "diagram.svg";
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  });

  bar.append(element("span", "", lang), toggle, save, copyButton(code));

  const pre = element("pre");
  pre.append(element("code", "", code));
  block.append(bar, element("div", "view"), pre);

  return block;
}

function figureKind(lang, code) {
  if (lang === "mermaid") return "mermaid";
  if (lang === "svg" || (["xml", "html"].includes(lang) && /^\s*(<\?xml[^>]*>\s*)?<svg\b/i.test(code))) return "svg";

  return "";
}

// ---- tables ----
// A row may be written with or without the outer pipes, and a cell may hold a
// literal `\|`; the header decides the column count, and the delimiter row
// (`:---:`) decides each column's alignment.
function splitRow(line) {
  const text = line.trim().replace(/^\|/, "").replace(/(?<!\\)\|$/, "");

  return text.split(/(?<!\\)\|/).map((cell) => cell.trim().replace(/\\\|/g, "|"));
}

function delimiterAligns(line) {
  if (!line.includes("|")) return null;

  const cells = splitRow(line);
  if (!cells.length || !cells.every((cell) => /^:?-+:?$/.test(cell))) return null;

  return cells.map((cell) =>
    cell.startsWith(":") && cell.endsWith(":") ? "center"
    : cell.endsWith(":") ? "right"
    : cell.startsWith(":") ? "left"
    : null);
}

function tableAt(lines, index) {
  return index + 1 < lines.length && lines[index].includes("|") && delimiterAligns(lines[index + 1]) !== null;
}

function alignCell(cell, alignment) {
  if (alignment) cell.style.textAlign = alignment;
}

function tableElement(header, rows, aligns) {
  const wrap = element("div", "table-wrap");
  const table = element("table");
  const head = element("thead");
  const headRow = element("tr");

  header.forEach((text, column) => {
    const th = element("th");
    alignCell(th, aligns[column]);
    inline(th, text);
    headRow.append(th);
  });

  head.append(headRow);

  const body = element("tbody");

  rows.forEach((cells) => {
    const row = element("tr");
    header.forEach((_, column) => {
      const td = element("td");
      alignCell(td, aligns[column]);
      inline(td, cells[column] ?? "");
      row.append(td);
    });
    body.append(row);
  });

  table.append(head, body);
  wrap.append(table);

  return wrap;
}

function tableBlock(parent, lines, index) {
  const aligns = delimiterAligns(lines[index + 1]);
  const header = splitRow(lines[index]);

  index += 2;
  const rows = [];

  while (index < lines.length && lines[index].includes("|") && lines[index].trim() && delimiterAligns(lines[index]) === null) {
    rows.push(splitRow(lines[index]));
    index += 1;
  }

  parent.append(tableElement(header, rows, aligns));

  return index;
}

// ---- block markdown ----
const FENCE = /^\s*(```+|~~~+)\s*([\w+#.-]*)/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const QUOTE = /^\s*>/;
const ITEM = /^\s*([-*+]|\d+[.)])\s+(.*)$/;
const MATH_OPEN = /^\s*(\$\$|\\\[)/;

function startsBlock(lines, index) {
  const line = lines[index];

  return !line.trim() || FENCE.test(line) || MATH_OPEN.test(line) || HEADING.test(line) || QUOTE.test(line) || tableAt(lines, index) || ITEM.test(line);
}

function codeFence(parent, lines, index, fence) {
  const body = [];
  index += 1;

  while (index < lines.length && !lines[index].trim().startsWith(fence[1])) {
    body.push(lines[index]);
    index += 1;
  }

  // A fence still open is a reply mid-stream: it is shown as code until it closes.
  const closed = index < lines.length;
  const code = body.join("\n");
  const lang = fence[2].toLowerCase();
  const kind = closed ? figureKind(lang, code) : "";

  parent.append(kind ? figureBlock(code, kind, lang) : codeBlock(code, lang));

  return index + 1;
}

// A display formula: $$ … $$ or \[ … \], on one line or across several.
// One that has not closed yet is mid-stream, and shows as its TeX.
function mathBlock(parent, lines, index) {
  const open = MATH_OPEN.exec(lines[index])[1];
  const close = open === "$$" ? "$$" : "\\]";
  const first = lines[index].trim().slice(open.length);
  const body = [];
  let closed = false;

  for (let line = first; ; line = lines[index]) {
    const end = line.indexOf(close);

    if (end !== -1) { body.push(line.slice(0, end)); closed = true; index += 1; break; }
    body.push(line);
    index += 1;
    if (index >= lines.length) break;
  }

  const tex = body.join("\n").trim();

  if (closed && tex) parent.append(formula(tex, true, "div"));
  else if (tex) parent.append(element("p", "math-pending", `${open} ${tex}`));

  return index;
}

function quoteBlock(parent, lines, index) {
  const quote = element("blockquote");
  const body = [];

  while (index < lines.length && QUOTE.test(lines[index])) {
    body.push(lines[index].replace(/^\s*>\s?/, ""));
    index += 1;
  }

  inline(quote, body.join("\n"));
  parent.append(quote);

  return index;
}

function listBlock(parent, lines, index) {
  const ordered = /\d/.test(ITEM.exec(lines[index])[1]);
  const list = element(ordered ? "ol" : "ul");

  while (index < lines.length) {
    const item = ITEM.exec(lines[index]);
    if (!item) break;

    const li = element("li");
    inline(li, item[2]);
    list.append(li);
    index += 1;
  }

  parent.append(list);

  return index;
}

function paragraph(parent, lines, index) {
  const body = [];

  while (index < lines.length && !startsBlock(lines, index)) {
    body.push(lines[index]);
    index += 1;
  }

  if (body.length) {
    const p = element("p");
    inline(p, body.join("\n"));
    parent.append(p);
  }

  return index;
}

export function markdown(parent, text) {
  const lines = text.split("\n");
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    const fence = FENCE.exec(line);

    if (fence) { index = codeFence(parent, lines, index, fence); continue; }
    if (MATH_OPEN.test(line)) { index = mathBlock(parent, lines, index); continue; }
    if (!line.trim()) { index += 1; continue; }

    const heading = HEADING.exec(line);

    if (heading) {
      const h = element(`h${Math.min(heading[1].length + 1, 4)}`);
      inline(h, heading[2]);
      parent.append(h);
      index += 1;
      continue;
    }

    if (QUOTE.test(line)) { index = quoteBlock(parent, lines, index); continue; }
    if (tableAt(lines, index)) { index = tableBlock(parent, lines, index); continue; }
    if (ITEM.test(line)) { index = listBlock(parent, lines, index); continue; }

    index = paragraph(parent, lines, index);
  }
}

// ---- sources ----
function sourcesPanel(summary, entries) {
  const details = element("details", "sources");
  details.append(element("summary", "", summary));

  const list = element("ul", "sources-list");

  entries.forEach((entry) => {
    const item = element("li", "source");
    const title = externalLink(entry.title || entry.domain || entry.url, entry.url);
    title.classList.add("source-title");
    item.append(title);

    const meta = [entry.domain, entry.published].filter(Boolean).join(" · ");
    if (meta) item.append(element("span", "source-meta", meta));
    list.append(item);
  });

  details.append(list);

  return details;
}

export function searchPanel(searches) {
  const count = searches.reduce((sum, search) => sum + search.results.length, 0);
  const summary = `searched: ${searches.map((search) => `“${search.query}”`).join(", ")} · ${count} sources`;

  return sourcesPanel(summary, searches.flatMap((search) => search.results));
}

export function readPanel(reads) {
  const summary = `read: ${reads.map((read) => `“${read.title}”`).join(", ")}`;

  return sourcesPanel(summary, reads);
}
