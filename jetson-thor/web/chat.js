const $ = (id) => document.getElementById(id);
const API = "/";
// A round may not generate without bound. Thinking can run away on a hard
// question, and a model can fall into a repeat loop; a token limit stops the
// engine, and the agent cuts the stream at its own cap as a second floor.
const MAX_ANSWER_TOKENS = 8192;
const store = {
  get(key, fallback) { try { const v = localStorage.getItem(key); return v === null ? fallback : JSON.parse(v); } catch { return fallback; } },
  set(key, value) { try { localStorage.setItem(key, JSON.stringify(value)); } catch {} },
};

let settings = store.get("settings", { key: "", system: "", temperature: "" });
let messages = store.get("messages", []);
let modelName = "";
let models = [];
let controller = null;

// ---- DOM helpers ----
function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

const SVG = "http://www.w3.org/2000/svg";
function cub(viewBox = "0 0 440 460") {
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", viewBox);
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS(SVG, "use");
  use.setAttribute("href", "#cub-art");
  svg.append(use);
  return svg;
}

const SUGGESTIONS = [
  ["Explain ownership in Rust", "with a small example that does not compile, then the fix"],
  ["Review my SQL schema", "paste it and ask what will hurt at a million rows"],
  ["What is new in CUDA 13?", "turn on Web first so it can search"],
  ["Plan a weekend project", "something I can build on a Raspberry Pi"],
];

let needsKey = false;

function headers() {
  const result = { "Content-Type": "application/json" };
  if (settings.key) result.Authorization = `Bearer ${settings.key}`;
  return result;
}

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

// ---- inline markdown: code, emphasis and links, all built from DOM nodes ----
// The pattern matches, in order, a code span, bold, italic, a markdown link,
// an <autolink>, a bare address, and a <br> the model writes to break a line
// inside a table cell. A bare address matters most in job answers, where the
// model writes the posting's URL as plain text.
const INLINE = /`(?<code>[^`\n]+)`|\*\*(?<bold>[^*\n]+)\*\*|\*(?<italic>[^*\n]+)\*|(?<!\w)_(?<under>[^_\n]+)_(?!\w)|\[(?<text>[^\]\n]+)\]\((?<href>https?:\/\/[^)\s]+)\)|<(?<auto>https?:\/\/[^>\s]+)>|(?<bare>(?:https?:\/\/|www\.)[^\s<>"'`]+)|(?<br><\/?[bB][rR]\s*\/?>)/g;

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

function codeBlock(code, lang) {
  const block = element("div", "code");
  const bar = element("div", "bar");
  const copy = element("button", "", "copy");
  copy.type = "button";
  copy.addEventListener("click", async () => {
    try { await navigator.clipboard.writeText(code); copy.textContent = "copied"; }
    catch { copy.textContent = "select + copy"; }
    setTimeout(() => { copy.textContent = "copy"; }, 1400);
  });
  bar.append(element("span", "", lang || "text"), copy);
  const pre = element("pre");
  const node = element("code");
  node.append(highlight(code, lang));
  pre.append(node);
  block.append(bar, pre);
  return block;
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

function startsBlock(lines, index) {
  const line = lines[index];
  return !line.trim() || FENCE.test(line) || HEADING.test(line) || QUOTE.test(line) || tableAt(lines, index) || ITEM.test(line);
}

function codeFence(parent, lines, index, fence) {
  const body = [];
  index += 1;
  while (index < lines.length && !lines[index].trim().startsWith(fence[1])) {
    body.push(lines[index]);
    index += 1;
  }
  index += 1;
  parent.append(codeBlock(body.join("\n"), fence[2].toLowerCase()));
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

function markdown(parent, text) {
  const lines = text.split("\n");
  let index = 0;
  while (index < lines.length) {
    const line = lines[index];
    const fence = FENCE.exec(line);
    if (fence) { index = codeFence(parent, lines, index, fence); continue; }
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

function searchPanel(searches) {
  const count = searches.reduce((sum, search) => sum + search.results.length, 0);
  const summary = `searched: ${searches.map((search) => `“${search.query}”`).join(", ")} · ${count} sources`;
  return sourcesPanel(summary, searches.flatMap((search) => search.results));
}

function readPanel(reads) {
  const summary = `read: ${reads.map((read) => `“${read.title}”`).join(", ")}`;
  return sourcesPanel(summary, reads);
}

// ---- thread ----
function part(name, node) {
  node.dataset.part = name;
  return node;
}

function renderMessage(message) {
  const wrap = element("div", `msg ${message.role}`);
  const body = element("div", "body");
  if (message.role === "assistant") {
    const avatar = element("div", "avatar");
    avatar.append(cub("100 66 220 220"));
    wrap.append(avatar);
  }
  if (message.role === "user") {
    body.textContent = message.content;
  } else {
    if (message.searches && message.searches.length) body.append(part("sources", searchPanel(message.searches)));
    if (message.reads && message.reads.length) body.append(part("read", readPanel(message.reads)));
    if (message.reasoning) {
      const details = element("details", "thinking");
      const blank = !message.streaming && !(message.content || "").trim();
      if (blank) details.open = true;
      details.append(element("summary", "", message.thinkingDone ? "thought" : "thinking…"), element("div", "text", message.reasoning));
      body.append(part("thinking", details));
    }
    if (message.streaming && !message.content && !message.reasoning) {
      const dots = element("div", "dots");
      dots.append(element("i"), element("i"), element("i"), element("span", "", $("web").checked ? "searching and thinking…" : "thinking…"));
      body.append(part("dots", dots));
    } else {
      const content = element("div");
      markdown(content, message.content || "");
      if (message.streaming) content.classList.add("cursor");
      body.append(part("content", content));
    }
    if (message.error) body.append(part("error", element("div", "error", message.error)));
    else if (!message.streaming && !(message.content || "").trim() && (message.reasoning || "").trim()) {
      const why = message.finish === "length" ? " It stopped at the token limit while it was still thinking." : "";
      body.append(part("blank", element("div", "blank", `The model thought but wrote no answer.${why} Its thinking is open above. Turn Think off, or set a smaller think budget in settings (⚙).`)));
    }
    if (message.stats) body.append(part("stats", element("div", "stats", message.stats)));
  }
  wrap.append(body);
  return wrap;
}

function render() {
  const thread = $("thread");
  thread.replaceChildren();
  if (needsKey) {
    const gate = element("div", "gate");
    const hero = element("div", "hero");
    hero.append(cub());
    gate.append(hero, element("h1", "", "The cub is invite-only for now"),
      element("p", "", "One Jetson Thor, four reply slots, shared by everyone who has a key."));

    const form = element("form");
    const field = element("input");
    field.type = "password"; field.placeholder = "Paste your access key"; field.autocomplete = "off";
    const go = element("button", "primary", "Unlock");
    form.append(field, go);
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      if (!field.value.trim()) return;
      settings.key = field.value.trim();
      store.set("settings", settings);
      connect();
    });
    gate.append(element("p", "step", "Already have a key?"), form);

    const register = element("form", "register");
    const who = element("input");
    who.placeholder = "Your name"; who.autocomplete = "name"; who.maxLength = 80;
    const mail = element("input");
    mail.type = "email"; mail.placeholder = "you@example.com"; mail.autocomplete = "email"; mail.maxLength = 200;
    const ask = element("button", "primary", "Request access");
    register.append(who, mail, ask);
    const outcome = element("small", "outcome");
    register.addEventListener("submit", async (event) => {
      event.preventDefault();
      outcome.textContent = "Sending…";
      try {
        const response = await fetch(`${API}request`, {
          method: "POST", headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: who.value.trim(), email: mail.value.trim() }),
        });
        outcome.textContent = response.ok
          ? "Request recorded. Now DM Arpan on LinkedIn or X so he can send your key."
          : `Could not record it: ${response.status} ${await response.text()}`;
      } catch {
        outcome.textContent = "The Thor did not answer. Try again in a moment.";
      }
    });

    const dm = element("span", "dm");
    const linkedin = element("a", "", "DM on LinkedIn");
    linkedin.href = "https://www.linkedin.com/in/arpan-pathak-272341424/";
    const twitter = element("a", "", "DM on X");
    twitter.href = "https://x.com/arpanpathak1996";
    [linkedin, twitter].forEach((link) => { link.target = "_blank"; link.rel = "noreferrer"; });
    dm.append(linkedin, twitter);

    const note = element("small");
    note.append("No key? Give your name and email, then DM Arpan. He approves you by hand and sends the key back: ");
    note.append(dm, ".");
    gate.append(element("p", "step", "New here? Ask for a key"), register, outcome, note);
    thread.append(gate);
    setTimeout(() => field.focus(), 0);
    return;
  }
  if (messages.length === 0) {
    const empty = element("div", "empty");
    const hero = element("div", "hero");
    hero.append(cub());
    const chips = element("div", "chips");
    SUGGESTIONS.forEach(([title, detail]) => {
      const chip = element("button");
      chip.type = "button";
      chip.append(element("b", "", title), element("span", "", detail));
      chip.addEventListener("click", () => { input.value = title; autosize(); input.focus(); });
      chips.append(chip);
    });
    empty.append(hero, element("h1", "", "Hi, I'm the Thor Tigress Cub."),
      element("p", "", `I'm ${modelName || "Nemotron 3 Nano"}, running on a Jetson AGX Thor in a home office. Ask me about code, systems, or anything else.`),
      chips);
    thread.append(empty);
    return;
  }
  messages.forEach((message) => thread.append(renderMessage(message)));
}

function nearBottom() {
  const scroller = $("scroller");
  return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 120;
}

// Parts that can be patched in place, so they never leave the page while a
// reply streams: a click on a node that was detached, even briefly, is lost.
const PATCHERS = {
  thinking(old, fresh) {
    old.querySelector("summary").textContent = fresh.querySelector("summary").textContent;
    old.querySelector(".text").textContent = fresh.querySelector(".text").textContent;
  },
};

function patchMessage(node, message) {
  const body = node.querySelector(".body");
  const old = new Map([...body.children].map((child) => [child.dataset.part, child]));
  const next = [...renderMessage(message).querySelector(".body").children].map((fresh) => {
    const kept = old.get(fresh.dataset.part);
    const patch = PATCHERS[fresh.dataset.part];
    if (!kept || !patch) return fresh;
    patch(kept, fresh);
    return kept;
  });
  [...body.children].filter((child) => !next.includes(child)).forEach((child) => child.remove());
  let cursor = body.firstChild;
  next.forEach((child) => {
    if (child === cursor) cursor = cursor.nextSibling;
    else body.insertBefore(child, cursor);
  });
}

function updateLast() {
  const stick = nearBottom();
  const last = $("thread").lastElementChild;
  const message = messages[messages.length - 1];
  if (last && last.classList.contains("assistant")) patchMessage(last, message);
  else $("thread").append(renderMessage(message));
  if (stick) $("scroller").scrollTop = $("scroller").scrollHeight;
}

function save() { store.set("messages", messages.map(({ streaming, ...rest }) => rest)); }

// ---- server ----
function modelLabel(id) {
  return id.split("/").pop().replace(/\.gguf$/, "").replace(/^NVIDIA-/, "").replace(/-(Q\d\w*)$/i, " · $1").replace(/-/g, " ");
}

function showServer(state, text) {
  $("status").className = `dot ${state}`;
  $("model").replaceChildren(element("option", "", text));
  $("model").disabled = true;
}

function showModels() {
  const picker = $("model");
  picker.replaceChildren(...models.map((id) => {
    const option = element("option", "", modelLabel(id));
    option.value = id;
    return option;
  }));
  const saved = store.get("model", "");
  picker.value = models.includes(saved) ? saved : models[0];
  picker.disabled = models.length < 2;
  modelName = modelLabel(picker.value);
  $("status").className = "dot ok";
}

function chosenModel() {
  return models.includes($("model").value) ? $("model").value : undefined;
}

async function connect() {
  try {
    const response = await fetch(`${API}v1/models`, { headers: headers() });
    needsKey = response.status === 401;
    const data = needsKey ? {} : await response.json();
    models = (data.data || []).map((model) => model.id);
    if (needsKey) showServer("bad", "access key needed");
    else if (models.length) showModels();
    else showServer("bad", "no model served");
    render();
  } catch {
    showServer("bad", "server not reachable");
  }
}

$("model").addEventListener("change", () => {
  store.set("model", $("model").value);
  modelName = modelLabel($("model").value);
  render();
});

// The one place a request is built. `max_tokens` bounds every round, so the
// engine stops a model that would otherwise generate until its context fills.
function chatRequest(history, thinking) {
  const body = {
    model: chosenModel(), messages: history, stream: true,
    stream_options: { include_usage: true },
    chat_template_kwargs: { enable_thinking: thinking },
    thor_web_search: $("web").checked,
    max_tokens: MAX_ANSWER_TOKENS,
  };
  if (settings.temperature !== "" && !Number.isNaN(Number(settings.temperature))) {
    body.temperature = Number(settings.temperature);
  }
  return body;
}

async function send(text) {
  messages.push({ role: "user", content: text });
  const reply = { role: "assistant", content: "", reasoning: "", streaming: true };
  messages.push(reply);
  render();
  $("scroller").scrollTop = $("scroller").scrollHeight;
  setBusy(true);

  const history = messages.slice(0, -1).filter((m) => !m.error && (m.role === "user" || m.content)).map(({ role, content }) => ({ role, content }));
  if (settings.system.trim()) history.unshift({ role: "system", content: settings.system.trim() });
  const body = chatRequest(history, $("think").checked);

  controller = new AbortController();
  const started = performance.now();
  let firstToken = 0;
  let lastToken = 0;
  try {
    const response = await fetch(`${API}v1/chat/completions`, {
      method: "POST", headers: headers(), body: JSON.stringify(body), signal: controller.signal,
    });
    if (!response.ok) throw new Error(response.status === 401 ? "Access key needed: open settings (⚙)." : `${response.status} ${await response.text()}`);
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let timings = null;
    let usage = null;
    let frame = 0;
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop();
      for (const line of lines) {
        if (!line.startsWith("data:")) continue;
        const payload = line.slice(5).trim();
        if (payload === "[DONE]") continue;
        const chunk = JSON.parse(payload);
        if (chunk.thor) {
          if (chunk.thor.search) (reply.searches = reply.searches || []).push(chunk.thor.search);
          if (chunk.thor.read) (reply.reads = reply.reads || []).push(chunk.thor.read);
          if (chunk.thor.error) reply.error = chunk.thor.error;
          continue;
        }
        const choice = chunk.choices && chunk.choices[0];
        const delta = (choice && choice.delta) || {};
        if (delta.content || delta.reasoning_content) {
          lastToken = performance.now();
          if (!firstToken) firstToken = lastToken;
        }
        if (delta.reasoning_content) reply.reasoning += delta.reasoning_content;
        if (delta.content) { reply.content += delta.content; reply.thinkingDone = true; }
        if (choice && choice.finish_reason) reply.finish = choice.finish_reason;
        if (chunk.timings) timings = chunk.timings;
        if (chunk.usage) usage = chunk.usage;
      }
      if (!frame) frame = requestAnimationFrame(() => { frame = 0; updateLast(); });
    }
    reply.thinkingDone = true;
    reply.stats = statsLine({ timings, usage, started, firstToken, lastToken, finish: reply.finish });
  } catch (error) {
    if (error.name === "AbortError") reply.stats = "stopped";
    else reply.error = error.message;
  } finally {
    reply.streaming = false;
    controller = null;
    setBusy(false);
    updateLast();
    save();
  }
}

// llama-server reports its own timings; other engines report only usage, so
// their speed is measured here, from the first token to the last.
function statsLine({ timings, usage, started, firstToken, lastToken, finish }) {
  const seconds = (ms) => `${(ms / 1000).toFixed(1)}s`;
  const total = seconds(performance.now() - started);
  const stop = finish && finish !== "stop" ? [finish === "length" ? "stopped at the token limit" : finish] : [];
  if (timings) {
    return [`${timings.predicted_n} tokens`, `${timings.predicted_per_second.toFixed(1)} tok/s`,
      `first token ${seconds(timings.prompt_ms)}`, total, ...stop].join(" · ");
  }
  const parts = [];
  if (usage) {
    parts.push(`${usage.completion_tokens} tokens`);
    const streaming = (lastToken - firstToken) / 1000;
    if (streaming > 0 && usage.completion_tokens > 1) parts.push(`${((usage.completion_tokens - 1) / streaming).toFixed(1)} tok/s`);
  }
  if (firstToken) parts.push(`first token ${seconds(firstToken - started)}`);
  return [...parts, total, ...stop].join(" · ");
}

function setBusy(busy) {
  $("send").textContent = busy ? "Stop" : "Send";
  $("send").type = busy ? "button" : "submit";
}

// ---- input ----
const input = $("input");
function autosize() { input.style.height = "auto"; input.style.height = `${input.scrollHeight}px`; }
input.addEventListener("input", autosize);
input.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey && !event.isComposing) { event.preventDefault(); $("composer").requestSubmit(); }
});
$("composer").addEventListener("submit", (event) => {
  event.preventDefault();
  if (controller) return;
  const text = input.value.trim();
  if (!text) return;
  input.value = "";
  autosize();
  send(text);
});
$("send").addEventListener("click", () => { if (controller) controller.abort(); });
document.addEventListener("keydown", (event) => { if (event.key === "Escape" && controller) controller.abort(); });
$("new").addEventListener("click", () => { if (controller) controller.abort(); messages = []; save(); render(); input.focus(); });
function applyTheme(name) {
  if (name === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = name;
}
$("theme").value = store.get("theme", "system");
applyTheme($("theme").value);
$("theme").addEventListener("change", () => { applyTheme($("theme").value); store.set("theme", $("theme").value); });

$("think").checked = store.get("think", false);
$("web").checked = store.get("web", false);
$("web").addEventListener("change", () => store.set("web", $("web").checked));
$("think").addEventListener("change", () => store.set("think", $("think").checked));

$("open-settings").addEventListener("click", () => {
  $("key").value = settings.key; $("system").value = settings.system; $("temperature").value = settings.temperature;
  $("settings").showModal();
});
$("settings").addEventListener("close", () => {
  if ($("settings").returnValue !== "save") return;
  settings = { key: $("key").value.trim(), system: $("system").value, temperature: $("temperature").value };
  store.set("settings", settings);
  connect();
});

render();
connect();
input.focus();
