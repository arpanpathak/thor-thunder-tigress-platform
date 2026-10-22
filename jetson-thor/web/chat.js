import { $, element, cub } from "./dom.js";
import { initAppearance, closeHistoryOnMobile } from "./appearance.js";
import { markdown, searchPanel, readPanel } from "./markdown.js";
import { LocalHistory, newConversation, conversationTitle, preferences as store } from "./history.js";
import { chatEvents } from "./stream.js";

const API = "/";

// A round may not generate without bound, and thinking must not go on forever.
// The think budget makes the engine force its own end-of-thinking tag, so the
// model answers; max_tokens bounds the round, and the agent cuts the stream as
// a last floor.
const MAX_ANSWER_TOKENS = 8192;
const DEFAULT_THINK_BUDGET = 1024;

let settings = store.get("settings", { key: "", system: "", temperature: "", thinkbudget: String(DEFAULT_THINK_BUDGET) });
if (settings.thinkbudget === undefined) settings.thinkbudget = String(DEFAULT_THINK_BUDGET);

const history = new LocalHistory();
let conversation = history.restore();
let messages = conversation.messages;
let models = [];

// Which engine serves each model, from /v1/models. Shown in the picker only when more
// than one engine is answering, so the same model on llama.cpp and on TensorRT Edge-LLM
// can be told apart before it is chosen.
const ENGINE_NAMES = { llamacpp: "llama.cpp", "tensorrt-edgellm": "TensorRT Edge-LLM" };
let modelEngines = {};
let controller = null;

const SUGGESTIONS = [
  ["01", "Take it apart", "Explain ownership in Rust with a small example that does not compile, then show the fix.", "Understand a concept"],
  ["02", "Make it better", "Help me review my SQL schema. Ask me to paste it, then look for problems at a million rows.", "Review code & systems"],
  ["03", "Build something", "Help me plan a weekend project I can build on a Raspberry Pi.", "Turn an idea into a plan"],
];

let needsKey = false;

function headers() {
  const result = { "Content-Type": "application/json" };

  if (settings.key) result.Authorization = `Bearer ${settings.key}`;

  return result;
}

// ---- thread ----
function part(name, node) {
  node.dataset.part = name;

  return node;
}

function renderMessage(message) {
  const wrap = element("article", `msg ${message.role}`);
  wrap.setAttribute("aria-label", message.role === "user" ? "Your message" : "Tigress reply");
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
    } else if (!message.streaming && !(message.content || "").trim() && message.stats !== "stopped") {
      body.append(part("blank", element("div", "blank", "The model returned no answer. Try again or choose another model.")));
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
    field.setAttribute("aria-label", "Access key");
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
    who.setAttribute("aria-label", "Your name");
    who.placeholder = "Your name"; who.autocomplete = "name"; who.maxLength = 80;
    const mail = element("input");
    mail.setAttribute("aria-label", "Email address");
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
    thread.append(welcome());

    return;
  }

  messages.forEach((message) => thread.append(renderMessage(message)));
}

// The opening composition uses the cub and Thor hardware as its identity.
function welcome() {
  const empty = element("section", "empty");
  const intro = element("div", "welcome-intro");
  const heading = element("h1");
  heading.append(element("span", "", "THOR"), element("span", "title-accent", "TIGRESS"));
  intro.append(element("span", "welcome-label", "LOCAL INTELLIGENCE / JETSON AGX THOR"), heading,
    element("p", "welcome-tagline", "Small cub. Serious curiosity."),
    element("p", "welcome-description", "Bring the stubborn bug, the half-built idea, the question you keep coming back to."));

  const art = element("div", "cub-stage");
  art.setAttribute("aria-hidden", "true");
  const orbit = element("div", "cub-orbit");
  const stripe = element("div", "cub-stripe");
  art.append(orbit, stripe, cub(), element("span", "art-label", "CUB / 01"), element("span", "art-caption", "A little thunder. A lot of possibility."));

  const prompts = element("div", "chips");
  SUGGESTIONS.forEach(([number, title, prompt, detail]) => {
    const button = element("button");
    button.type = "button";
    const label = element("span", "prompt-label");
    label.append(element("b", "", title), element("span", "prompt-detail", detail));
    button.append(element("span", "prompt-number", number), label, element("span", "prompt-arrow", "↗"));
    button.addEventListener("click", () => { input.value = prompt; autosize(); input.focus(); });

    prompts.append(button);
  });

  const caption = element("div", "welcome-caption");
  caption.append(element("span", "", "YOUR NEXT CONVERSATION"), element("span", "", "PICK A STARTING POINT ↓"));

  empty.append(intro, art, caption, prompts);

  return empty;
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

function save() {
  conversation.messages = messages;
  conversation.draft = input.value;
  conversation.updatedAt = Date.now();

  const saved = history.save(conversation);
  showStorageStatus();
  renderHistory();

  return saved;
}

function showStorageStatus() {
  $("storage-status").textContent = history.error;
  $("storage-status").hidden = !history.error;
}

function renderHistory() {
  const chats = history.list().filter((chat) => chat.messages.length || chat.draft.trim());
  const list = $("history-list");

  list.replaceChildren();
  $("chat-title").textContent = messages.length ? conversationTitle(conversation) : "Thor Tigress Cub";
  $("export-chat").disabled = !messages.length && !input.value;

  if (!chats.length) list.append(element("p", "history-empty", "A fresh start. Your conversations will appear here."));
  chats.forEach((chat) => {
    const row = element("div", "history-row");
    const open = element("button", "history-item");
    open.setAttribute("aria-current", String(chat.id === conversation.id));
    open.title = conversationTitle(chat);
    open.append(element("span", "history-title", conversationTitle(chat)),
      element("small", "", new Date(chat.updatedAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })));
    open.disabled = Boolean(controller);

    open.addEventListener("click", () => {
      if (chat.id === conversation.id) { closeHistoryOnMobile(); return; }
      if (!save()) return;

      activateConversation(history.read(chat.id) || chat);
    });

    const remove = element("button", "delete-chat", "×");
    remove.setAttribute("aria-label", `Delete ${conversationTitle(chat)}`);
    remove.disabled = Boolean(controller);

    remove.addEventListener("click", () => {
      $("delete-chat").dataset.id = chat.id;
      $("delete-chat").showModal();
    });

    row.append(open, remove);
    list.append(row);
  });

  showStorageStatus();
}

function activateConversation(next) {
  conversation = next;
  messages = next.messages;
  input.value = next.draft;

  history.save(conversation);
  render();
  renderHistory();
  autosize();

  closeHistoryOnMobile();
  $("scroller").scrollTop = $("scroller").scrollHeight;
}

function exportChat() {
  const text = messages.map((message) => `## ${message.role === "user" ? "You" : "Thor Tigress Cub"}\n\n${message.content}${message.error ? `\n\nError: ${message.error}` : ""}`).join("\n\n---\n\n");
  const draft = input.value ? `\n\n## Unsent draft\n\n${input.value}` : "";
  const url = URL.createObjectURL(new Blob([`# ${conversationTitle(conversation)}\n\n${text}${draft}\n`], { type: "text/markdown" }));
  const link = element("a");
  link.href = url;
  link.download = `thor-chat-${new Date().toISOString().slice(0, 10)}.md`;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

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
  const engines = new Set(models.map((id) => modelEngines[id]).filter(Boolean));
  picker.replaceChildren(...models.map((id) => {
    const engine = ENGINE_NAMES[modelEngines[id]] || modelEngines[id];
    const option = element("option", "", modelLabel(id) + (engines.size > 1 && engine ? ` (${engine})` : ""));
    option.value = id;
    return option;
  }));

  const saved = store.get("model", "");

  picker.value = models.includes(saved) ? saved : models[0];
  picker.disabled = models.length < 2;
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
    modelEngines = Object.fromEntries((data.data || []).map((model) => [model.id, model.owned_by]));

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
  render();
});

// The one place a request is built. `max_tokens` bounds every round; when
// thinking is on, the budget makes the engine force its own end-of-thinking
// tag, so an overlong think ends with an answer. The message is empty on
// purpose: a sentence there is injected into the model's own text, and the
// model then answers the sentence instead of the question.
//
// The two budget fields are llama-server options. TensorRT Edge-LLM refuses a
// body field it does not know with "Extra inputs are not permitted", so they are
// sent to llama-server only. Turning Think on for a model an engine serves still
// works, through chat_template_kwargs; it just has no cap on its thinking.
//
// vLLM has no budget either. Qwen3.8's template thinks at effort xhigh by
// default and spends all of max_tokens on a mid-sized coding question, so the
// page asks vLLM for medium effort instead.
function servedByLlamaCpp() {
  const engine = modelEngines[chosenModel()];
  return !engine || engine === "llamacpp";
}

function servedByVllm() {
  return modelEngines[chosenModel()] === "vllm";
}

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

  const budget = Number(settings.thinkbudget);

  if (servedByLlamaCpp() && thinking && settings.thinkbudget !== "" && Number.isInteger(budget) && budget >= 0) {
    body.reasoning_budget_tokens = budget;
    body.reasoning_budget_message = "";
  }

  if (servedByVllm() && thinking) body.chat_template_kwargs.reasoning_effort = "medium";

  return body;
}

async function send(text) {
  messages.push({ role: "user", content: text });
  const reply = { role: "assistant", content: "", reasoning: "", streaming: true };
  messages.push(reply);

  save();
  render();
  $("scroller").scrollTop = $("scroller").scrollHeight;

  controller = new AbortController();
  const signal = controller.signal;
  const deadline = setTimeout(() => controller?.abort(new Error("The response exceeded ten minutes. The partial reply was saved.")), 600000);
  setBusy(true);

  const history = messages.slice(0, -1).filter((m) => !m.error && (m.role === "user" || m.content)).map(({ role, content }) => ({ role, content }));
  const formatting = "Write readable Markdown. Separate logical ideas with blank lines. Use short paragraphs and spaced lists when helpful; keep related points together. Preserve code indentation and blank lines between logical code sections.";

  // The models' training ends before today, so without the date they take
  // recent dates (a job ending last month) for typos or the future.
  const today = `Today's date is ${new Date().toLocaleDateString("en-US", { weekday: "long", year: "numeric", month: "long", day: "numeric" })}.`;

  // Web search is its own mode: the server writes the system line for the hunt,
  // and adding the formatting line above it makes the model keep hunting until
  // the rounds run out. Keep the old shape there; keep formatting in plain chat.
  if ($("web").checked) {
    history.unshift({ role: "system", content: [today, settings.system.trim()].filter(Boolean).join("\n\n") });
  } else {
    history.unshift({ role: "system", content: [today, formatting, settings.system.trim()].filter(Boolean).join("\n\n") });
  }

  const body = chatRequest(history, $("think").checked);

  const started = performance.now();
  let firstToken = 0;
  let lastToken = 0;
  let frame = 0;

  try {
    const response = await fetch(`${API}v1/chat/completions`, {
      method: "POST", headers: headers(), body: JSON.stringify(body), signal,
    });

    if (!response.ok) throw new Error(response.status === 401 ? "Access key needed: open settings (⚙)." : `${response.status} ${await response.text()}`);

    let timings = null;
    let usage = null;
    let savedAt = performance.now();

    for await (const chunk of chatEvents(response.body, signal)) {
      if (chunk.error) throw new Error(chunk.error.message || "The model server returned an error.");

      if (chunk.thor) {
        if (chunk.thor.search) (reply.searches = reply.searches || []).push(chunk.thor.search);
        if (chunk.thor.read) (reply.reads = reply.reads || []).push(chunk.thor.read);
        if (chunk.thor.error) reply.error = chunk.thor.error;

        continue;
      }

      const choice = chunk.choices && chunk.choices[0];
      const delta = (choice && choice.delta) || {};
      const reasoning = delta.reasoning_content || delta.reasoning;

      if (delta.content || reasoning) {
        lastToken = performance.now();
        if (!firstToken) firstToken = lastToken;
      }

      if (reasoning) reply.reasoning += reasoning;
      if (delta.content) { reply.content += delta.content; reply.thinkingDone = true; }
      if (choice && choice.finish_reason) reply.finish = choice.finish_reason;
      if (chunk.timings) timings = chunk.timings;
      if (chunk.usage) usage = chunk.usage;

      if (performance.now() - savedAt > 1500) { save(); savedAt = performance.now(); }
      if (!frame) frame = requestAnimationFrame(() => { frame = 0; updateLast(); });
    }

    reply.thinkingDone = true;
    reply.stats = statsLine({ timings, usage, started, firstToken, lastToken, finish: reply.finish });
  } catch (error) {
    const reason = signal.aborted ? signal.reason : error;

    if (reason.name === "AbortError") reply.stats = "stopped";
    else reply.error = reason.message;
  } finally {
    clearTimeout(deadline);
    cancelAnimationFrame(frame);

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
  $("send").firstElementChild.textContent = busy ? "Stop" : "Send";
  $("send").querySelector("use").setAttribute("href", busy ? "#icon-stop" : "#icon-arrow");
  $("send").title = busy ? "Stop response · Esc" : "Send · Shift+Enter";
  $("send").setAttribute("aria-label", busy ? "Stop response" : "Send message");
  $("send").classList.toggle("busy", busy);

  ["new", "open-settings", "model"].forEach((id) => { $(id).disabled = busy || (id === "model" && models.length < 2); });
  $("scroller").setAttribute("aria-busy", String(busy));
  renderHistory();
  $("send").type = busy ? "button" : "submit";
}

// ---- input ----
const input = $("input");

function autosize() { input.style.height = "auto"; input.style.height = `${input.scrollHeight}px`; }
// The height limit depends on the window (and the on-screen keyboard), so re-fit on resize.
window.addEventListener("resize", autosize);

let draftTimer;

input.addEventListener("input", () => {
  autosize();
  clearTimeout(draftTimer);
  draftTimer = setTimeout(save, 300);
});
window.addEventListener("pagehide", save);
document.addEventListener("visibilitychange", () => { if (document.hidden) save(); });
window.addEventListener("storage", () => { renderHistory(); });
input.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && event.shiftKey && !event.isComposing) { event.preventDefault(); $("composer").requestSubmit(); }
});
$("composer").addEventListener("submit", (event) => {
  event.preventDefault();

  if (controller || needsKey) return;
  const text = input.value.trim();
  if (!text) return;

  input.value = "";
  autosize();

  send(text);
});
$("send").addEventListener("click", () => { if (controller) controller.abort(); });
document.addEventListener("keydown", (event) => { if (event.key === "Escape" && controller && !document.querySelector("dialog[open]") && !document.body.classList.contains("history-open")) controller.abort(); });
$("new").addEventListener("click", () => {
  if (controller) return;
  if (!messages.length && !input.value.trim()) { closeHistoryOnMobile(); input.focus(); return; }
  if (!save()) return;
  activateConversation(newConversation());
  input.focus();
});
$("export-chat").addEventListener("click", exportChat);
$("open-privacy").addEventListener("click", () => $("privacy").showModal());
$("delete-chat").addEventListener("close", () => {
  if ($("delete-chat").returnValue !== "delete") return;
  const id = $("delete-chat").dataset.id;
  if (history.remove(id) && id === conversation.id) activateConversation(history.list()[0] || newConversation());
  renderHistory();
});
$("think").checked = store.get("think", false);
$("web").checked = store.get("web", false);
$("web").addEventListener("change", () => store.set("web", $("web").checked));
$("think").addEventListener("change", () => store.set("think", $("think").checked));

$("open-settings").addEventListener("click", () => {
  $("key").value = settings.key; $("system").value = settings.system; $("temperature").value = settings.temperature;
  $("thinkbudget").value = settings.thinkbudget;
  $("settings").showModal();
});
$("settings").addEventListener("close", () => {
  if ($("settings").returnValue !== "save") return;
  settings = { key: $("key").value.trim(), system: $("system").value, temperature: $("temperature").value, thinkbudget: $("thinkbudget").value.trim() };
  store.set("settings", settings);
  connect();
});

initAppearance();
input.value = conversation.draft;
render();
renderHistory();
autosize();
connect();
