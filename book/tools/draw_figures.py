"""Draws the book's figures with the kit in figkit.py.

    python3 book/tools/draw_figures.py book/src/figures            every figure
    python3 book/tools/draw_figures.py book/src/figures NAME ...   only these
"""
import sys
from pathlib import Path

from figkit import (EDGE, INK, KINDS, MUTED, card, cylinder, icon, legend, person, save, sheet, stacked, swatches, tag,
                    text, wire, zone)

OUT = Path(sys.argv[1])
FIGURES = {}


def figure(draw):
    FIGURES[draw.__name__.replace("_", "-")] = draw
    return draw


def lines(x, y, bodies, cls="note", fill=None, step=17):
    return "\n".join(text(x, y + step * row, body, cls, fill=fill) for row, body in enumerate(bodies))


# ---- chapter 1: high-level design ----

@figure
def platform_overview():
    planned = lambda x, w, name, sub: card(x, 420, w, 76, "store", name, sub, dashed=True)
    parts = [
        card(40, 110, 226, 60, "store", "Chat export", "384 conversations", glyph="doc"),
        card(40, 190, 226, 60, "store", "Curated sets", "55 + 74 pairs, 37 DPO", glyph="doc"),
        card(40, 270, 226, 60, "store", "Book repository", "chapters and code files", glyph="doc"),
        card(300, 184, 240, 72, "service", "thor-hammer-trainer", ["data generator, Rust", "pairs, splits, filters"], glyph="rust"),
        cylinder(600, 178, 170, 84, "store", "train.jsonl", ["3,165 examples", "1.76M tokens"]),
        card(830, 190, 200, 60, "client", "Review page", "reading, slop flags", glyph="browser"),
        wire([(266, 140), (283, 140), (283, 208), (300, 208)]),
        wire([(266, 220), (300, 220)]),
        wire([(266, 300), (283, 300), (283, 232), (300, 232)]),
        wire([(540, 220), (600, 220)]),
        wire([(770, 220), (830, 220)]),

        planned(40, 180, "QLoRA training", ["Unsloth + TRL", "on the Thor GPU"]),
        planned(240, 186, "Merge and quantize", ["LoRA folded into", "base weights"]),
        planned(446, 200, "TensorRT-LLM engine", "built for sm_110"),
        planned(666, 170, "trtllm-serve", ["OpenAI-style HTTP", "on localhost"]),
        planned(856, 184, "Human evaluation", ["base vs fine-tuned,", "blind A/B"]),
        wire([(685, 262), (685, 372), (130, 372), (130, 420)], "idle"),
        wire([(220, 458), (240, 458)], "idle"), wire([(426, 458), (446, 458)], "idle"),
        wire([(646, 458), (666, 458)], "idle"), wire([(836, 458), (856, 458)], "idle"),
        wire([(948, 420), (948, 340), (1050, 340), (1050, 84), (420, 84), (420, 184)], "idle",
             label="verdicts choose the data for the next round", on=3),
        legend(40, 532, [("service", "built and run on real data"), ("store", "a file or a set of files")]),
        f'  <rect x="470" y="522" width="14" height="14" rx="4" fill="{KINDS["store"][1]}" stroke="{KINDS["store"][0]}" stroke-width="1.3" stroke-dasharray="3 2"/>',
        text(491, 533.5, "designed, not built yet", "note"),
    ]
    save(OUT / "platform-overview.svg", 1080, 556,
         "Three sources feed thor-hammer-trainer, which writes train.jsonl. train.jsonl feeds the review page and the "
         "planned chain: QLoRA training, merge and quantize, TensorRT-LLM engine, trtllm-serve. trtllm-serve feeds "
         "the planned human evaluation, whose verdicts feed the next data round.",
         "Components and data flow", "Solid cards are built. Dashed cards are designed and not built yet.", parts)


@figure
def thor_memory():
    left, per_gb = 60, 840 / 128
    bar = lambda y, gb, colour: f'  <rect x="{left}" y="{y}" width="{gb * per_gb:.1f}" height="22" rx="5" fill="{colour}" filter="url(#lift)"/>'
    amber, green, blue = KINDS["store"][0], KINDS["model"][0], KINDS["client"][0]
    parts = [
        stacked(left, 96, 840, 40, [(13, "#8A99AB", "OS"), (115, KINDS["service"][0], "about 115 GB usable for a training job")], 128),
        text(left, 156, "0", "mono dim"), text(left + 840, 156, "128 GB", "mono dim", "end"),
        text(left, 192, "Reported peak memory per training method (from the project notes, not measured here):", "note"),
        bar(206, 28, green), text(left + 28 * per_gb + 12, 221.5, "QLoRA, 27B model: about 28 GB", "note", fill=green),
        bar(240, 42, blue), text(left + 42 * per_gb + 12, 255.5, "Full fine-tune, 4B model: about 42 GB", "note", fill=blue),
        bar(274, 50, amber), text(left + 50 * per_gb + 12, 289.5, "LoRA, 9B model: about 50 GB", "note", fill=amber),
        lines(left, 330, ["Bandwidth: 273 GB/s. Each training step reads all the weights, so step time is bound by memory",
                          "bandwidth, not by compute."]),
    ]
    save(OUT / "thor-memory.svg", 960, 372,
         "A bar of 128 GB with about 115 GB usable. Reported peak memory: QLoRA on a 27B model about 28 GB, full "
         "fine-tuning of a 4B model about 42 GB, LoRA on a 9B model about 50 GB.",
         "128 GB of unified memory, shared by the CPU and the GPU",
         "The three bars below are drawn on the same scale as the 128 GB bar.", parts)


@figure
def crates_today():
    parts = [
        card(40, 114, 216, 72, "store", "Sources", ["books and docs", "chat export, own pairs"], glyph="doc"),
        card(300, 114, 250, 72, "service", "thor-hammer-trainer", "builds the training set", glyph="rust"),
        cylinder(610, 108, 180, 84, "store", "data/train.jsonl", "13,114 records"),
        card(850, 84, 170, 56, "service", "reinforcer", "review page", glyph="rust"),
        card(850, 160, 170, 56, "service", "lasso", "conversations", glyph="rust"),
        wire([(256, 150), (300, 150)]), wire([(550, 150), (610, 150)]),
        wire([(790, 144), (820, 144), (820, 112), (850, 112)]),
        wire([(790, 156), (820, 156), (820, 188), (850, 188)]),

        card(40, 268, 216, 64, "store", "Model answers", "one JSON line each", glyph="doc"),
        card(300, 264, 250, 72, "service", "spark", ["slop and the five", "Rust rules"], glyph="rust"),
        cylinder(610, 262, 180, 76, "store", "Scores", "per answer"),
        wire([(256, 300), (300, 300)]), wire([(550, 300), (610, 300)]),
    ]
    save(OUT / "crates-today.svg", 1060, 372,
         "Sources feed thor-hammer-trainer, which writes data/train.jsonl, read by the reinforcer review page and by "
         "lasso. Model answers go to spark, which writes scores.",
         "The crates and the files between them", "Two lines of work that share no file: building the data, and scoring answers.", parts)


# ---- chapter 3: the training data ----

@figure
def training_set_composition():
    chat, book, code, curated = KINDS["client"][0], KINDS["service"][0], KINDS["outside"][0], KINDS["store"][0]
    parts = [
        text(60, 104, "Tokens (1.76M)", "name"),
        stacked(60, 114, 840, 40, [(1178903, chat, "chat 1,178,903 (67%)"), (414747, book, "book 414,747 (24%)"),
                                   (134264, code, "code"), (29480, curated, "")]),
        text(60, 194, "Examples (3,165)", "name"),
        stacked(60, 204, 840, 40, [(2218, chat, "chat 2,218 (70%)"), (690, book, "book 690 (22%)"),
                                   (128, code, ""), (129, curated, "")]),
        swatches(60, 280, [(chat, "chat"), (book, "book"), (code, "code: 128 examples, 134,264 tokens (8%)"),
                           (curated, "curated: readability 55, clever_vs_readable 74")]),
        lines(60, 310, ["The curated sets are 29,480 tokens (1.7%); the training plan repeats them about 5 times per epoch.",
                        "Token counts are estimates at four characters per token."]),
    ]
    save(OUT / "training-set-composition.svg", 960, 350,
         "Two stacked bars. Tokens: chat 67 percent, book 24 percent, code 8 percent, curated sets 1.7 percent. "
         "Examples: chat 70 percent, book 22 percent, code 4 percent, curated sets 4 percent.",
         "Training set composition by source", "The same set measured two ways: by tokens, and by examples.", parts)


# ---- chapter 4: the data generator ----

@figure
def trainer_modules():
    readers = [(40, "readability.rs", "### Instruction / Response"), (275, "chat.rs", "serde types, pairing"),
               (510, "book.rs", "sections at # ## ###"), (745, "code.rs", "header comment = request")]
    parts = [card(340, 96, 320, 64, "service", "main.rs", "build_training_set: read, filter, write")]
    for x, name, sub in readers:
        centre = x + 107.5
        parts += [card(x, 220, 215, 64, "service", name, sub),
                  wire([(500, 160), (500, 190), (centre, 190), (centre, 220)]),
                  wire([(centre, 284), (centre, 344)], "idle")]
    parts += [
        card(40, 344, 920, 64, "outside", "example.rs", "Example { instruction, response, source, origin } and SkipReason"),
        cylinder(160, 440, 200, 74, "store", "train.jsonl", "serde_json, one per line"),
        card(420, 446, 240, 62, "service", "report.rs", "stats.md: counts and skips"),
        card(720, 446, 240, 62, "danger", "error.rs", "DataError: Io, Json"),
        wire([(260, 408), (260, 440)]), wire([(540, 408), (540, 446)]),
        tag(40, 536, "solid: calls", "service"), tag(150, 536, "dotted: produces", "idle"),
        text(300, 548.6, "main.rs writes train.jsonl and calls report.rs. Every module returns DataError.", "note"),
    ]
    save(OUT / "trainer-modules.svg", 1000, 574,
         "main.rs calls readability.rs, chat.rs, book.rs and code.rs. Each produces Example and SkipReason values "
         "defined in example.rs. main.rs writes train.jsonl and calls report.rs. error.rs defines DataError.",
         "thor-hammer-trainer modules", "One reader per kind of source. All four produce the same two types.", parts)


@figure
def chat_pairing():
    parts = [
        text(40, 100, "MESSAGES, IN ORDER", "tab"), text(560, 100, "PAIRS, THEN EXAMPLES", "tab"),
        sheet(40, 112, 430, 76, "client", "HUMAN", [("", "What is a futex?")]),
        sheet(40, 202, 430, 130, "service", "ASSISTANT", [
            ("dim", "thinking: dropped"), ("", 'text: "A fast userspace mutex."'), ("dim", "tool_use: dropped"),
            ("", 'text: "The kernel only steps in on contention."')]),
        sheet(40, 346, 430, 94, "client", "HUMAN", [("", "Look at this"), ("dim", "files: screen.PNG (not in the export)")]),
        sheet(40, 454, 430, 76, "service", "ASSISTANT", [("", "The picture shows a graph.")]),

        sheet(560, 136, 400, 150, "good", "PAIR 1 · TURN 1", [
            ("", "question: What is a futex?"), ("", "answer: the two text blocks, joined"), ("dim", "asks_about_image: false")],
            ["Ok(Example)   origin: c1/1"]),
        sheet(560, 372, 400, 132, "danger", "PAIR 2 · TURN 2", [
            ("", "question: Look at this"), ("dim", "asks_about_image: true")], ["Err(SkipReason::AboutImage)"]),

        wire([(470, 150), (515, 150), (515, 199), (560, 199)]),
        wire([(470, 267), (515, 267), (515, 223), (560, 223)]),
        wire([(470, 393), (515, 393), (515, 426), (560, 426)]),
        wire([(470, 492), (515, 492), (515, 450), (560, 450)]),
        lines(40, 560, ["A human message extends an unanswered pair; otherwise it starts a new pair.",
                        "An assistant message adds its text blocks to the current answer; hidden blocks are dropped."]),
    ]
    save(OUT / "chat-pairing.svg", 1000, 596,
         "Four messages: a human question, an assistant message with thinking, two text blocks and a tool call, a "
         "human message with an image, and an assistant answer. They become pair 1, kept, and pair 2, skipped with "
         "AboutImage.",
         "Chat pairing: four messages, two pairs, one example", "Grey lines are parts of a message the pairing drops.", parts)


@figure
def book_splitting():
    bar = lambda top, bottom: f'  <rect x="47" y="{top}" width="4" height="{bottom - top}" rx="2" fill="{KINDS["good"][0]}"/>'
    parts = [
        sheet(40, 96, 400, 270, "store", "CHAPTER MARKDOWN", [
            ("strong", "# 13 An LRU cache"), ("", "Intro text."),
            ("strong", "## 13.1 What an LRU cache does"), ("", "It evicts."), ("dim", "```bash"),
            ("trap", "# not a heading"), ("dim", "```"), ("strong", "## Summary"), ("", "We built one.")],
            ["Inside the fence a # line is text."]),
        bar(141, 174), bar(178, 264), bar(268, 300),
        sheet(560, 96, 400, 92, "good", "EXAMPLE 1", [("", 'Introduce "An LRU cache".'), ("dim", "response: Intro text.")]),
        sheet(560, 204, 400, 128, "good", "EXAMPLE 2", [
            ("", 'Explain "What an LRU cache does"'), ("", 'from "An LRU cache".'),
            ("dim", "response: It evicts, plus the whole"), ("dim", "code block, # line included")]),
        sheet(560, 348, 400, 92, "good", "EXAMPLE 3", [
            ("", 'Explain "Summary" from "An LRU cache".'), ("dim", "response: We built one.")]),
        wire([(440, 157), (500, 157), (500, 142), (560, 142)]),
        wire([(440, 222), (500, 222), (500, 268), (560, 268)]),
        wire([(440, 287), (480, 287), (480, 394), (560, 394)]),
        lines(40, 400, ["Each green bar on the left is one section, cut at its heading.", "Section numbers such as 13.1 are stripped from headings.",
                        "A section over 12,000 characters is cut again at ###."]),
    ]
    save(OUT / "book-splitting.svg", 1000, 470,
         "A chapter with a title, an intro, section 13.1 containing a bash code block with the line '# not a "
         "heading', and a Summary section. Cuts occur before 13.1 and before Summary, not inside the code block. "
         "Three examples result.",
         "Chapter splitting: headings inside code blocks are text", "One chapter becomes one example per heading.", parts)


# ---- chapter 5: the review server ----

@figure
def review_server():
    parts = [
        zone(230, 104, 746, 316, "This Thor · 127.0.0.1"),
        card(24, 150, 160, 80, "outside", "Other machine", ["on the network"]),
        wire([(184, 190), (226, 190)], "danger"),
        lines(24, 254, ["Cannot connect: the", "socket binds 127.0.0.1."], fill=KINDS["danger"][0]),
        sheet(254, 130, 320, 246, "client", "BROWSER · REVIEW.HTML", [
            ("", "parse 3,165 JSON lines"), ("", "search and source filter"), ("", "20 examples per page"),
            ("", "highlight emails, phone numbers,"), ("", "keys, passwords"), ("", "flag as AI slop, with a note")],
            ["All text is set with textContent:", "HTML in the data is not executed."]),
        sheet(644, 130, 308, 132, "service", "REVIEW · RUST, STD ONLY", [
            ("", "GET  / /train.jsonl /slop.jsonl"), ("", "GET  /flags   POST /flags"), ("dim", "anything else → 404")]),
        cylinder(698, 312, 200, 84, "store", "data/ and labels/", "read on every request"),
        wire([(574, 170), (644, 170)]), wire([(644, 214), (574, 214)]),
        text(609, 160, "request", "note", "middle"), text(609, 234, "response", "note", "middle"),
        wire([(798, 262), (798, 312)]),
    ]
    save(OUT / "review-server.svg", 1000, 444,
         "On 127.0.0.1, the browser requests / and /train.jsonl from the review server. The server reads "
         "data/train.jsonl on each request. Another machine cannot connect.",
         "Review server: routes and the loopback boundary", "Everything inside the tinted area is one machine talking to itself.", parts)


# ---- chapter 8: local models ----

@figure
def opencode_tunnel():
    parts = [
        zone(24, 108, 430, 216, "Your machine"), zone(600, 108, 376, 216, "Jetson AGX Thor"),
        card(48, 176, 176, 80, "client", "opencode", ["reads, edits,", "runs commands"], glyph="terminal"),
        card(276, 150, 156, 50, "idle", "127.0.0.1:8079"), card(276, 232, 156, 50, "idle", "127.0.0.1:8888"),
        card(630, 147, 320, 56, "service", "llama-server :8079", "Nemotron", glyph="stack"),
        card(630, 229, 320, 56, "service", "SearXNG :8888", "web search", glyph="searxng"),
        wire([(224, 204), (250, 204), (250, 175), (276, 175)]),
        wire([(224, 228), (250, 228), (250, 257), (276, 257)]),
        wire([(432, 175), (630, 175)], label="SSH tunnel"), wire([(432, 257), (630, 257)], label="SSH tunnel"),
        text(48, 352, "The tunnel uses your SSH login. Nothing new is exposed on the Thor.", "note"),
    ]
    save(OUT / "opencode-tunnel.svg", 1000, 372,
         "opencode on your machine calls 127.0.0.1:8079 and 127.0.0.1:8888; an SSH tunnel carries both to "
         "llama-server and SearXNG on the Thor.",
         "OpenCode on your machine, the model on the Thor", "Two local ports, each carried to the Thor inside one SSH connection.", parts)


# ---- chapter 9: web chat ----

@figure
def cub_architecture():
    parts = [
        card(40, 110, 160, 64, "client", "Browser", "or agent", glyph="browser"),
        card(330, 110, 200, 64, "outside", "voltforge.tech", "forwarding page", glyph="doc"),
        card(680, 110, 230, 64, "outside", "Tailscale Funnel", "HTTPS to the Thor", glyph="tailscale"),
        wire([(200, 142), (330, 142)]), wire([(530, 142), (680, 142)]),
        text(265, 132, "1. short address", "note", "middle"), text(605, 132, "2. the .ts.net name", "note", "middle"),

        zone(24, 256, 992, 250, "Jetson AGX Thor · everything on 127.0.0.1"),
        card(60, 300, 280, 96, "service", "thor-tigress-agent", [":8080 · key, APIs", "model picker", "SearXNG :8888 when Web is on"], glyph="swap"),
        card(480, 280, 260, 80, "service", "llama.cpp", [":8079 · GGUF", "Nano · Lightning · Qwen 27B"], glyph="stack"),
        card(480, 384, 260, 80, "model", "Edge-LLM", [":8081 · NVFP4", "Qwen3.6 35B A3B"], glyph="nvidia"),
        wire([(795, 174), (795, 214), (200, 214), (200, 300)], label="3. every request", on=1),
        wire([(340, 336), (410, 336), (410, 320), (480, 320)]),
        wire([(340, 360), (410, 360), (410, 424), (480, 424)]),
        lines(772, 366, ["The picker's model id", "chooses the engine."]),
        lines(60, 430, ["thor-tigress-serve loads and unloads on either engine,", "and checks the free memory first."]),
    ]
    save(OUT / "cub-architecture.svg", 1040, 530,
         "A browser or agent opens voltforge.tech, whose forwarding page sends it to the .ts.net address. Every "
         "request then goes through Tailscale Funnel to thor-tigress-agent on the Thor at :8080, which holds the "
         "key, the model picker and the APIs, and calls SearXNG at :8888 for web search. The picker's model id "
         "decides which engine runs the reply: the llama.cpp router at :8079 for the GGUF models, or TensorRT "
         "Edge-LLM at :8081 for Qwen3.6 35B A3B in NVFP4.",
         "How a request reaches a model", "The short address is only a signpost. Every request after it goes straight to the Thor.", parts)


# ---- chapter 10: security ----

@figure
def keyring_flow():
    red = KINDS["danger"][0]
    parts = [
        card(40, 120, 260, 80, "client", "Browser and agents", ["a personal key in a header", "Authorization: Bearer"], glyph="key"),
        card(40, 260, 260, 80, "outside", "Tailscale Funnel", ["HTTPS, so the header", "is encrypted in transit"], glyph="tailscale"),
        wire([(170, 200), (170, 260)]),
        zone(350, 100, 666, 356, "Jetson AGX Thor"),
        cylinder(380, 130, 300, 80, "store", "~/.config/thor-chat/keyring", "the active keys, re-read on change"),
        cylinder(710, 130, 290, 80, "store", "~/.config/thor-chat/api-key", "the key the agent forwards with"),
        card(400, 260, 260, 80, "service", "thor-tigress-agent", ["compares each key", "in constant time"], glyph="swap"),
        card(740, 252, 250, 96, "service", "llama-server", ["checks the one", "service key", ":8079 or :8081"], glyph="stack"),
        wire([(300, 300), (400, 300)]), wire([(660, 300), (740, 300)]),
        wire([(530, 210), (530, 260)], "idle"), wire([(865, 210), (865, 252)], "idle"),
        text(400, 366, "no match: 401", "note", fill=red), text(740, 374, "a key with no match: 401", "note", fill=red),
        lines(380, 412, ["A key that matches is forwarded with the agent's own key,", "and llama-server checks that one again."]),
    ]
    save(OUT / "keyring-flow.svg", 1040, 480,
         "Browsers and agents send a personal key as Authorization: Bearer over HTTPS through Tailscale Funnel. "
         "thor-tigress-agent compares it, in constant time, with the active keys in the encrypted keyring and with "
         "the operator's single key, and answers 401 when it is wrong. A match is forwarded to llama-server with "
         "the agent's own key, which checks again.",
         "How a personal key is checked", "Two checks, two keys: yours at the agent, the agent's at the model server.", parts)


# ---- chapter 11: training and serving (planned) ----

@figure
def training_serving():
    steps = [
        ("1. QLoRA", ["base frozen in 4 bit,", "small adapters train", "in 16 bit"], "adapter weights", "tens of MB"),
        ("2. Merge", ["adapters folded into", "full-precision base"], "merged model", "16 bit, 9B ≈ 18 GB"),
        ("3. Quantize", ["same mode for base", "and fine-tuned"], "quantized model", "smaller, faster"),
        ("4. Build engine", ["TensorRT-LLM", "for sm_110"], "engine files", "tied to this GPU"),
        ("5. Serve", ["trtllm-serve", "HTTP, localhost"], "endpoint", "/v1/chat/completions"),
    ]
    parts = []
    for index, (name, sub, artifact, detail) in enumerate(steps):
        x = 40 + 206 * index
        parts += [card(x, 104, 176, 96, "store", name, sub, dashed=True),
                  cylinder(x, 256, 176, 72, "store", artifact, detail),
                  wire([(x + 88, 200), (x + 88, 256)], "idle")]
        if index:
            parts.append(wire([(x - 30, 152), (x, 152)], "idle"))
    parts += [
        wire([(334, 328), (334, 366), (540, 366), (540, 328)], "danger", label="compare these two", on=1),
        lines(40, 404, ["Diagnosis: compare the merged model with the quantized model before changing data or training settings.",
                        "Every reported result records the build command, quantization mode, maximum sequence length, batch size,",
                        "tokens per second and peak memory. An unsupported mode on sm_110 stops the run."]),
    ]
    save(OUT / "training-serving.svg", 1060, 462,
         "Five planned steps: QLoRA, merge, quantize, build engine, serve, each producing one artifact. A diagnosis "
         "path compares the merged model with the quantized model.",
         "Stage 2 steps and artifacts (planned)", "Each step, top, leaves one artifact, bottom. None of this is built yet.", parts)


# ---- chapter 12: human evaluation (planned) ----

@figure
def human_eval_loop():
    plan = lambda x, y, w, h, name, sub, glyph=None: card(x, y, w, h, "store", name, sub, glyph=glyph, dashed=True)
    parts = [
        plan(40, 170, 190, 64, "Prompt list", "written by hand", "doc"),
        plan(300, 110, 220, 64, "Base model", "same quantization", "chip"),
        plan(300, 230, 220, 64, "Fine-tuned model", "same quantization", "chip"),
        plan(590, 150, 210, 104, "Side by side", ["answer A, answer B", "order shuffled,", "names hidden"]),
        plan(860, 170, 160, 64, "Reviewer", "A, B or tie"),
        cylinder(580, 318, 230, 80, "store", "verdicts.jsonl", "prompt, both answers, verdict"),
        wire([(230, 196), (265, 196), (265, 142), (300, 142)], "idle"),
        wire([(230, 208), (265, 208), (265, 262), (300, 262)], "idle"),
        wire([(520, 142), (555, 142), (555, 190), (590, 190)], "idle"),
        wire([(520, 262), (555, 262), (555, 214), (590, 214)], "idle"),
        wire([(800, 202), (860, 202)], "idle"),
        wire([(940, 234), (940, 358), (810, 358)], "idle"),
        wire([(580, 358), (135, 358), (135, 234)], "idle", label="losses by topic choose sources for the next round", on=0),
    ]
    save(OUT / "human-eval-loop.svg", 1060, 430,
         "A prompt list is sent to the base model and the fine-tuned model. Answers are shown as A and B in "
         "shuffled order with model names hidden. The reviewer picks A, B or tie. Verdicts are written to "
         "verdicts.jsonl and used to select data for the next round.",
         "Blind pairwise comparison (planned)", "The reviewer never sees which model wrote which answer.", parts)


# ---- chapter 13: evaluation design ----

@figure
def eval_pipeline():
    parts = [
        card(70, 110, 240, 60, "store", "eval/held_out.jsonl", "frozen before Stage 2", glyph="doc"),
        card(70, 186, 240, 60, "store", "eval/public/", "MultiPL-E, GSM8K, XSTest…", glyph="doc"),
        card(70, 262, 240, 60, "store", "eval/probes.jsonl", "privacy leakage prompts", glyph="doc"),
        card(70, 340, 240, 56, "idle", "overlap_report.md", "13-gram check vs train.jsonl"),
        wire([(70, 140), (46, 140), (46, 368), (70, 368)], "idle"),
        card(380, 176, 200, 80, "service", "runner", ["same prompts, decoding,", "seeds for both models"]),
        wire([(310, 140), (345, 140), (345, 204), (380, 204)]), wire([(310, 216), (380, 216)]),
        wire([(310, 292), (345, 292), (345, 228), (380, 228)]),
        card(650, 120, 210, 64, "model", "base endpoint", "trtllm-serve", glyph="chip"),
        card(650, 250, 210, 64, "model", "fine-tuned endpoint", "trtllm-serve"),
        wire([(580, 204), (615, 204), (615, 152), (650, 152)]), wire([(580, 228), (615, 228), (615, 282), (650, 282)]),
        cylinder(900, 170, 150, 92, "store", "answers.jsonl", "runs/<run_id>/"),
        wire([(860, 152), (880, 152), (880, 204), (900, 204)]), wire([(860, 282), (880, 282), (880, 228), (900, 228)]),

        card(380, 420, 410, 100, "service", "automatic scorers", [
            "compile and tests · five rules · false claims", "slop density · knowledge judge (κ-validated)",
            "refusal · over-refusal · leakage"]),
        card(870, 430, 180, 80, "client", "reviewer", ["blind A/B,", "slop spans"]),
        cylinder(70, 420, 240, 100, "store", "scores.jsonl", ["summary.md", "rates with 95% CI"]),
        wire([(975, 262), (975, 430)]), wire([(975, 262), (975, 380), (585, 380), (585, 420)]),
        wire([(380, 470), (310, 470)]),
        wire([(960, 510), (960, 548), (190, 548), (190, 520)]),
        lines(70, 578, ["Every run writes runs/<run_id>/manifest.json: model, quantization, build command, max sequence length, batch size,",
                        "decoding settings, seed, git commit, tokens per second, peak memory."]),
    ]
    save(OUT / "eval-pipeline.svg", 1080, 616,
         "Frozen inputs (held_out.jsonl, public benchmarks, privacy probes) feed a runner that sends identical "
         "requests to the base and fine-tuned trtllm-serve endpoints. Answers go to automatic scorers and to the "
         "reviewer. Scores and a summary with 95 percent intervals are written per run. An overlap report checks "
         "held-out items against train.jsonl.",
         "Evaluation: frozen inputs, two endpoints, per-item scores, intervals (planned)",
         "Both models get the same prompts, the same decoding and the same seeds.", parts)


@figure
def median_answers():
    red, blue, grey = KINDS["danger"][0], KINDS["client"][0], KINDS["idle"][0]
    # (bar height in px at 25 tokens a pixel, colour, reply time, unwrap, false claim, compiles)
    answers = [
        (18, None, "stopped", 1, 0, 0), (24, red, "13 s", 1, 1, 0), (18, None, "stopped", 1, 1, 0),
        (36, red, "18 s", 1, 1, 0), (179, red, "86 s", 1, 1, 0), (44, red, "22 s", 1, 1, 0),
        (219, blue, "105 s", 0, 0, 0), (82, blue, "40 s", 0, 0, 0), (101, red, "49 s", 1, 1, 0),
        (56, blue, "28 s", 0, 0, 0), (106, blue, "52 s", 0, 0, 0), (56, red, "30 s", 1, 1, 0),
        (41, blue, "21 s", 0, 0, 1),
    ]
    base, left = 330, 250
    parts = [text(40, 100, "tokens per reply", "note")]
    for tokens in (0, 2000, 4000):
        y = base - tokens / 25
        parts += [f'  <line x1="{left - 12}" y1="{y}" x2="984" y2="{y}" stroke="{EDGE}" stroke-width="1"/>',
                  text(left - 20, y + 4, f"{tokens:,}", "note", "end")]
    rows = [(364, "answer"), (392, "unwrap() in the code"), (416, "claims no unwrap, falsely"), (440, "compiles")]
    parts += [text(40, y, body, "note") for y, body in rows]
    for index, (height, colour, time, unwrap, claim, builds) in enumerate(answers):
        x = left + 56 * index
        style = f'fill="{colour}" filter="url(#lift)"' if colour else f'fill="#FFFFFF" stroke="{grey}" stroke-dasharray="4 3"'
        parts += [f'  <rect x="{x}" y="{base - height}" width="40" height="{height}" rx="5" {style}/>',
                  text(x + 20, base - height - 8, time, "note", "middle"),
                  text(x + 20, 364, str(index + 1), "name", "middle")]
        for y, flag, good in ((392, unwrap, False), (416, claim, False), (440, builds, True)):
            mark, fill = ("yes", KINDS["good"][0] if good else red) if flag else ("·", MUTED)
            parts.append(text(x + 20, y, mark, "note", "middle", fill))
    parts += [
        swatches(40, 478, [(red, "the answer uses unwrap()"), (blue, "no unwrap()")]),
        f'  <rect x="420" y="468" width="14" height="14" rx="4" fill="#FFFFFF" stroke="{grey}" stroke-dasharray="3 2"/>',
        text(441, 479.5, "stopped by the reader", "note"),
        text(40, 506, "Seconds are total reply time. The 4,465 and 5,482 token replies are mostly hidden reasoning.", "note"),
    ]
    save(OUT / "median-answers.svg", 1020, 526,
         "One bar per Nemotron answer, height is tokens. Eight of thirteen answers use unwrap, seven of them while "
         "saying they do not. Only answer 13, a copy of the user's own code, compiles.",
         "Thirteen answers to one running-median request", "One bar per answer, in the order they were given.", parts)


@figure
def history_pressure():
    turns = [("client", "you: write a running median, no unwrap"), ("danger", "cub: code with unwrap()"),
             ("client", "you: I asked not to use unwrap"), ("danger", "cub: code with unwrap()"),
             ("idle", "7 more rounds like these"), ("client", "you: your own correct code, pasted")]
    parts = [zone(24, 112, 468, 350, "The request for answer 12")]
    parts += [card(48, 140 + 52 * row, 420, 42, kind, name, dashed=kind == "idle") for row, (kind, name) in enumerate(turns)]
    parts += [
        card(560, 150, 260, 96, "model", "Nemotron 3 Nano", ["30B weights, 3B active", "per token, temperature 1.0"], glyph="nvidia"),
        card(560, 330, 260, 96, "danger", "Answer 12", ["your code, rewritten, with", "min_high.unwrap() back in it"]),
        card(880, 142, 180, 112, "danger", "No system prompt", ["the rules exist only", "as complaints", "between answers"], dashed=True),
        wire([(492, 198), (560, 198)]), wire([(690, 246), (690, 330)]), wire([(880, 198), (820, 198)], "danger"),
    ]
    save(OUT / "history-pressure.svg", 1084, 486,
         "The request for answer 12 holds every earlier answer, most with unwrap, and the rules only as short user "
         "complaints. With no system prompt and temperature 1.0, Nemotron rewrote the pasted code and put unwrap back.",
         "Why the cub kept repeating itself", "By answer 12 the conversation held far more unwrap() than requests to avoid it.", parts)


@figure
def teacher_pipeline():
    parts = [
        card(40, 130, 190, 80, "idle", "Gaps G1 to G6", ["seen in the chat", "section 7.7"]),
        card(290, 130, 240, 80, "store", "train/teacher/*.md", ["60 conversations", "written by the teacher"], glyph="doc"),
        card(590, 105, 220, 130, "service", "teacher", ["clippy -D warnings", "-W clippy::pedantic", "tests must pass",
                                                        "spark: rules, slop,", "false claims"], glyph="rust"),
        cylinder(870, 96, 180, 64, "store", "teacher.jsonl", "60 chats, for SFT"),
        cylinder(870, 176, 180, 64, "store", "teacher_preferences", "7 pairs, for DPO"),
        card(870, 256, 180, 72, "danger", "teacher.md", ["numbers and", "every problem"]),
        wire([(230, 170), (290, 170)]), wire([(530, 170), (590, 170)]),
        wire([(810, 158), (840, 158), (840, 128), (870, 128)]),
        wire([(810, 170), (840, 170), (840, 208), (870, 208)]),
        wire([(810, 182), (826, 182), (826, 292), (870, 292)], "danger"),
        wire([(960, 328), (960, 368), (410, 368), (410, 210)], "danger", label="an entry fails: fix and rerun", on=1),

        card(40, 404, 490, 92, "store", "Runtime fixes, planned and independent of training", [
            "default system prompt ·", "spark check and one retry before showing code ·",
            "Think off and lower temperature for code"], dashed=True),
        card(590, 414, 200, 72, "store", "Same prompts, again", ["scored by spark", "planned"], dashed=True),
        card(850, 418, 200, 64, "store", "Stage 2 QLoRA", "planned", dashed=True),
        wire([(1050, 128), (1076, 128), (1076, 450), (1050, 450)], "idle"),
        wire([(850, 450), (790, 450)], "idle"),
    ]
    save(OUT / "teacher-pipeline.svg", 1100, 520,
         "Gaps from the chat become conversations in train/teacher. The teacher binary builds every Rust block with "
         "clippy pedantic, runs its tests and runs spark; passing entries go to teacher.jsonl and "
         "teacher_preferences.jsonl, failures to teacher.md for fixing. Training and re-evaluation are planned.",
         "From the gaps to checked training data", "Nothing reaches the training files without building, passing its tests and passing spark.", parts)


# ---- chapter 16: bring your own agent ----

@figure
def agent_wiring():
    parts = [
        card(40, 120, 250, 80, "client", "Your agent", ["openbatrangs, Claude Code,", "OpenCode, Python, curl"], glyph="terminal"),
        card(350, 120, 220, 80, "outside", "Tailscale Funnel", ["https://…ts.net", "key in a header"], glyph="tailscale"),
        zone(630, 96, 390, 300, "Jetson AGX Thor"),
        card(654, 112, 342, 96, "service", "thor-tigress-agent", [":8080 checks the key;", "/v1/messages: thinking", "off unless asked"], glyph="swap"),
        card(654, 276, 342, 96, "service", "llama-server", [":8079", "OpenAI and", "Anthropic APIs"], glyph="stack"),
        card(40, 286, 250, 80, "client", "Your machine", ["with SSH access", "to the Thor"], glyph="terminal"),
        card(350, 286, 220, 80, "idle", "SSH tunnel", ["127.0.0.1:8080", "no public hop"]),
        wire([(290, 160), (350, 160)]), wire([(570, 148), (654, 148)]),
        wire([(290, 326), (350, 326)]), wire([(570, 326), (612, 326), (612, 176), (654, 176)]),
        wire([(825, 208), (825, 276)]),
    ]
    save(OUT / "agent-wiring.svg", 1044, 420,
         "Agents call the .ts.net address through Tailscale Funnel with the key in a header; thor-tigress-agent "
         "checks it and passes requests to llama-server. A machine with SSH access reaches the same server through "
         "an SSH tunnel.",
         "How agents reach the Thor", "Two ways in, and both arrive at the same door: the agent on port 8080.", parts)


# ---- chapter 19: tool calling ----

@figure
def tool_loop():
    parts = [
        card(40, 150, 170, 64, "client", "Browser", "Web switch on", glyph="browser"),
        zone(240, 104, 590, 440, "Jetson AGX Thor"),
        card(270, 142, 240, 80, "service", "thor-tigress-agent", ["up to eight rounds;", "the first requires a tool"], glyph="swap"),
        card(580, 142, 220, 80, "model", "llama-server", ["Nemotron or Qwen:", "a tool call or an answer"]),
        wire([(210, 182), (270, 182)]), wire([(510, 170), (580, 170)]), wire([(580, 196), (510, 196)]),
        lines(580, 246, ["→ messages and tools", "← a tool call, or the answer"]),
        card(270, 300, 270, 80, "service", "web_search", ["SearXNG :8888", "6 results, with dates"], glyph="search"),
        card(270, 420, 270, 80, "service", "fetch_page_content_recursive", "a page and its own links"),
        card(580, 420, 220, 80, "danger", "Address checks", ["public https only", "every hop, one lookup"]),
        wire([(390, 222), (390, 300)]),
        wire([(300, 222), (300, 266), (254, 266), (254, 460), (270, 460)]),
        wire([(540, 340), (870, 340)]), wire([(540, 460), (580, 460)]), wire([(800, 460), (870, 460)]),
        card(870, 300, 190, 80, "outside", "Search engines", ["DuckDuckGo, Google", "(Brave suspended)"]),
        card(870, 420, 190, 80, "outside", "The internet", ["the cited page,", "then its links"]),
    ]
    save(OUT / "tool-loop.svg", 1084, 566,
         "The browser asks thor-tigress-agent with Web on. The agent sends messages and tools to llama-server; "
         "Nemotron or Qwen calls web_search, which asks SearXNG and the search engines for recent results with "
         "dates, or calls fetch_page_content_recursive, which checks that every hop is a public https address and "
         "reads a page and its own links. After at most eight tool rounds the model has to answer.",
         "Tool calling in the chat: search and read", "The model asks, the agent does. Nothing the model writes reaches the internet unchecked.", parts)


@figure
def recursive_read():
    link = lambda x, y, name, sub: card(x, y, 176, 56, "outside", name, sub)
    parts = [
        zone(24, 108, 716, 400, "The page's own site, and nothing outside it"),
        card(46, 274, 210, 64, "client", "cited page", "depth 0 · title, text", glyph="doc"),
        link(316, 178, "a link on it", "depth 1"), link(316, 378, "a link on it", "depth 1"),
        link(544, 138, "another link", "depth 2 · no further"), link(544, 278, "another link", "depth 2 · no further"),
        link(544, 418, "another link", "depth 2 · no further"),
        wire([(256, 294), (286, 294), (286, 206), (316, 206)]), wire([(256, 318), (286, 318), (286, 406), (316, 406)]),
        wire([(492, 194), (518, 194), (518, 166), (544, 166)]), wire([(492, 218), (518, 218), (518, 306), (544, 306)]),
        wire([(492, 406), (518, 406), (518, 446), (544, 446)]),
        sheet(780, 108, 250, 252, "store", "LIMITS, PER ANSWER", [
            ("", "6 pages in all"), ("", "2 hops deep"), ("", "12,000 characters a page"), ("", "24,000 characters in all"),
            ("", "10 seconds a page"), ("", "2 MB a page"), ("", "https text pages only")],
            ["A page that fails is", "left out of the answer."]),
    ]
    save(OUT / "recursive-read.svg", 1060, 532,
         "fetch_page_content_recursive reads the cited page at depth 0, then up to three of its own links at depth "
         "1, then their links at depth 2, but no further. It follows only links on the cited page's own site, and "
         "a panel lists the limits: 6 pages in all, 2 hops deep, 12,000 characters a page, 24,000 in all, 10 "
         "seconds and 2 MB a page, https text pages only.",
         "Reading the cited page, and the links on it", "Two hops from the page the answer cited, and never off its site.", parts)


# ---- chapter 20: memory, context and slots ----

@figure
def slots():
    people = ["Browser", "Browser", "Agent", "Browser", "Agent"]
    parts = [zone(250, 108, 770, 366, "Jetson AGX Thor · llama-server, --parallel 4")]
    for row, name in enumerate(people):
        y = 143 + 62 * row
        parts += [card(40, y - 23, 150, 46, "client", name),
                  wire([(190, y), (220, y), (220, 267), (280, 267)])]
    parts += [lines(40, 446, ["any number of people,", "one key"]),
              card(280, 217, 170, 100, "idle", "Queue", ["requests wait here", "when all 4 slots", "are busy"]),
              cylinder(830, 180, 170, 174, "model", "Model weights", ["one copy, 32 GB,", "shared by all slots;", "every step reads",
                                                                      "the weights it uses", "once for all slots"])]
    for slot in range(4):
        y = 162 + 84 * slot
        parts += [card(520, y - 35, 230, 70, "service", f"Slot {slot}", ["up to 1,048,576 tokens", "borrowed for one reply"]),
                  wire([(450, 267), (485, 267), (485, y), (520, y)]),
                  wire([(750, y), (790, y), (790, 267), (830, 267)], "idle")]
    save(OUT / "slots.svg", 1044, 500,
         "Any number of people send requests with the same key. llama-server keeps a queue and four slots; each "
         "request borrows a free slot for one reply, up to 1,048,576 tokens, and gives it back. All slots share one "
         "copy of the model weights.",
         "People, slots and the model", "A slot is borrowed for one reply and handed back. The weights are loaded once.", parts)


@figure
def memory_map():
    green, blue, grey, pale = KINDS["model"][0], KINDS["client"][0], "#8A99AB", "#A9BACB"
    explain = lambda x, name, sub: card(x, 290, 280, 104, "outside", name, sub)
    dividers = "\n".join(f'  <line x1="{280 + 41.25 * n:.1f}" y1="116" x2="{280 + 41.25 * n:.1f}" y2="172" stroke="#FFFFFF" stroke-width="1.5"/>' for n in (1, 2, 3))
    parts = [
        text(60, 104, "0", "mono dim"), text(940, 104, "128 GB, shared by CPU and GPU", "mono dim", "end"),
        stacked(60, 116, 880, 56, [(32, green, "weights · 32 GB"), (24, blue, ""), (17, grey, "OS, buffers"),
                                   (55, pale, "free and file cache · about 49 GB available")], 128),
        dividers,
        text(362.5, 192, "context · 24 GiB · slots 0 to 3", "note", "middle"),
        f'  <path d="M60,214 V222 H445 V214" fill="none" stroke="{MUTED}" stroke-width="1.3"/>',
        text(252.5, 242, "llama-server: 57.8 GB measured", "note", "middle"),
        explain(60, "Per token of context", ["6 KiB: only 6 of 52 layers", "are attention layers, each", "with 2 key/value heads of 128"]),
        explain(360, "Per slot", ["1,048,576 tokens × 6 KiB", "= 6 GiB, plus a small", "fixed state for the other layers"]),
        explain(660, "Reserved at start", ["for all 4 slots, used or not:", "4 × 6 GiB = 24 GiB;", "today's longest chat: 41,674 tokens"]),
    ]
    save(OUT / "memory-map.svg", 1000, 420,
         "A 128 GB bar: about 32 GB model weights, 24 GiB reserved for the context of four slots of one million "
         "tokens, the rest of the 73 GB in use by buffers, the OS and other programs, and about 49 GB available. "
         "Below, the arithmetic: 6 KiB per token, 6 GiB per slot, 24 GiB reserved.",
         "Where the Thor's memory goes while serving", "The context is paid for at startup, whether or not anyone uses it.", parts)


@figure
def kv_per_token():
    green, blue = KINDS["model"][0], KINDS["client"][0]
    models = [("Nemotron 3 Nano 30B-A3B", "6 attention layers × 2 KV heads × 128", 6, green),
              ("Llama 3 8B", "32 layers × 8 KV heads × 128", 128, blue),
              ("Llama 3 70B", "80 layers × 8 KV heads × 128", 320, blue)]
    parts = [text(60, 104, "Memory per token of context (16-bit keys and values)", "note"),
             text(940, 104, "for 1,048,576 tokens", "note", "end")]
    for row, (name, shape, kib, colour) in enumerate(models):
        y, wide = 126 + 78 * row, kib * 420 / 320
        parts += [text(60, y + 18, name, "name"), text(60, y + 36, shape, "note"),
                  f'  <rect x="360" y="{y + 4}" width="{wide:.1f}" height="34" rx="5" fill="{colour}" filter="url(#lift)"/>',
                  text(360 + wide + 12, y + 26, f"{kib} KiB", "name", fill=colour),
                  text(940, y + 26, f"{kib} GiB", "name", "end")]
    parts.append(lines(60, 384, ["The Thor has about 115 GB usable. A 1M-token conversation fits only because most of Nemotron's layers",
                                 "keep a fixed-size state instead of a cache that grows with every token."]))
    save(OUT / "kv-per-token.svg", 1000, 420,
         "Memory per token of context: Nemotron 3 Nano 6 KiB, Llama 3 8B 128 KiB, Llama 3 70B 320 KiB. For one "
         "million tokens: 6 GiB, 128 GiB, 320 GiB.",
         "Why a 1M-token context fits on the Thor", "The bars are to scale. Nemotron's is the short one.", parts)


# ---- chapter 21: model serving ----

@figure
def model_router():
    parts = [
        card(40, 110, 190, 72, "client", "Web chat", "picker sends model", glyph="browser"),
        card(40, 230, 190, 72, "client", "Claude Code,", ["OpenCode", 'model: "nemotron"']),
        card(40, 350, 190, 72, "client", "curl, SDKs", "any id or alias", glyph="terminal"),
        card(290, 214, 230, 104, "service", "thor-tigress-agent", [":8080", "key check, web search", "passes model through"], glyph="swap"),
        card(580, 204, 220, 124, "service", "llama-server router", [":8079", "routes by model or alias", "--models-max 2", "unknown name: 400"]),
        card(850, 100, 226, 88, "model", "Nemotron 3 Nano", ["loaded · 4 × 1M", "nemotron, nemotron-think"]),
        card(850, 222, 226, 88, "idle", "Nemotron 3.5 Lightning", ["on disk · not loaded", "thor-tigress-serve load 2"], dashed=True),
        card(850, 344, 226, 88, "idle", "a model you load", ["from ~/models/gguf", "thor-tigress-serve load"], dashed=True),
        wire([(230, 146), (260, 146), (260, 254), (290, 254)]), wire([(230, 266), (290, 266)]),
        wire([(230, 386), (260, 386), (260, 278), (290, 278)]),
        wire([(520, 266), (580, 266)]),
        wire([(800, 254), (825, 254), (825, 144), (850, 144)]),
        wire([(800, 266), (850, 266)], "idle"),
        wire([(800, 278), (825, 278), (825, 388), (850, 388)], "idle"),
        cylinder(580, 392, 220, 92, "store", "~/.config/thor-chat/", ["env → models.ini", "+ models.local.ini"]),
        wire([(690, 392), (690, 328)], "idle"),
        text(566, 444, "written by thor-tigress-serve", "note", "end"),
        lines(40, 516, ["Each loaded model is its own llama-server process on a private port.",
                        "thor-tigress-serve refuses a third; the router alone would evict one."]),
    ]
    save(OUT / "model-router.svg", 1100, 552,
         "Clients send requests with a model name to thor-tigress-agent on port 8080, which passes them to the "
         "llama-server router on 8079. The router sends each request to the model process with that name or alias. "
         "Nemotron 3 Nano is loaded; Nemotron 3.5 Lightning and other GGUF files under ~/models/gguf stay on disk "
         "until loaded with thor-tigress-serve, which writes the router's model list in ~/.config/thor-chat.",
         "How a request reaches a model", "The router picks the model process by the name in the request.", parts)


@figure
def model_memory():
    nano, lightning, grey, pale = KINDS["service"][0], KINDS["model"][0], "#8A99AB", "#A9BACB"
    rows = [("Nano 57.8 GB", "what llama-server reported at start with 4 slots × 1,048,576 tokens (chapter \"Memory, context and slots\")"),
            ("Lightning 35.4 GB", "MemAvailable rose from 24.8 to 60.2 GB on ./serve.sh unload lightning, 2026-10-06"),
            ("OS and others 4.8 GB", "the remainder"),
            ("Available 24.8 GB", "keep at least 8 GB free; below that the OOM killer may stop thor-chat")]
    parts = [stacked(60, 100, 900, 56, [(57.8, nano, "Nemotron 3 Nano · 57.8 GB"), (35.4, lightning, "Lightning · 35.4 GB"),
                                        (4.8, grey, ""), (24.8, pale, "available · 24.8 GB")])]
    for row, (label, why) in enumerate(rows):
        parts += [text(60, 196 + 24 * row, label, "name"), text(240, 196 + 24 * row, why, "note")]
    parts.append(text(60, 306, "Tried and failed: Lightning at 4 × 1M next to the Nano ran out of memory on its first reply.", "note",
                      fill=KINDS["danger"][0]))
    save(OUT / "model-memory.svg", 1020, 330,
         "A bar of the Thor's 122.8 GB: Nemotron 3 Nano 57.8 GB, Lightning 35.4 GB, OS and other programs 4.8 GB, "
         "24.8 GB available.",
         "The Thor's memory with both models loaded (122.8 GB)", "Measured on 2026-10-06, with llama.cpp serving both.", parts)


# ---- chapter 22: TensorRT Edge-LLM ----

@figure
def edge_llm_design():
    parts = [
        card(30, 206, 210, 64, "client", "Web chat, agents", "model: id or alias", glyph="browser"),
        card(300, 186, 240, 104, "service", "thor-tigress-agent", [":8080", "--engine per other engine", "routes by model id"], glyph="swap"),
        card(610, 100, 230, 96, "service", "llama.cpp router", [":8079 · GGUF", "Nemotron 3 Nano Q8_0", "default model"], glyph="stack"),
        card(610, 280, 230, 112, "model", "Edge-LLM server", [":8081 · TensorRT engine", "Qwen3.6-35B-A3B NVFP4", "service thor-edge-llm"], glyph="nvidia"),
        cylinder(896, 100, 170, 96, "store", "GGUF files", ["~/models/gguf/", "downloaded as is"]),
        cylinder(890, 274, 180, 124, "store", "engine files", ["~/models/edge-llm/cache", "built on the Thor", "for sm_110 only"]),
        card(300, 362, 240, 80, "idle", "thor-tigress-serve", ["EDGE_MODEL; list,", "load, unload"], glyph="gnubash"),
        wire([(240, 238), (300, 238)]),
        wire([(540, 226), (575, 226), (575, 148), (610, 148)]), wire([(540, 250), (575, 250), (575, 336), (610, 336)]),
        wire([(840, 148), (896, 148)], "idle"), wire([(840, 336), (890, 336)], "idle"),
        text(40, 478, "Both engines share the Thor's 122.8 GB. thor-tigress-serve manages both, and checks free memory before loading on either.", "note"),
    ]
    save(OUT / "edge-llm-design.svg", 1100, 500,
         "thor-tigress-agent on 8080 routes each request by model id to either the llama.cpp router on 8079, "
         "serving Nemotron 3 Nano from a GGUF file, or the TensorRT Edge-LLM server on 8081, serving "
         "Qwen3.6-35B-A3B from engines built on the Thor. thor-tigress-serve manages both.",
         "Two engines behind one agent", "The request's model id decides which engine answers.", parts)


@figure
def edge_llm_pipeline():
    steps = [("1. Checkpoint", ["NVIDIA NVFP4", "on Hugging Face", "19.3 GB"], "huggingface"),
             ("2. Export", ["tensorrt-edgellm-", "export → ONNX", "PyTorch container"], None),
             ("3. Build", ["llm_build on the", "Thor's GPU", "time: to measure"], "nvidia"),
             ("4. Serve", ["experimental", "OpenAI server", "port 8081"], None),
             ("5. Measure", ["same prompts as", "llama.cpp: speed,", "memory, spark"], None)]
    parts = []
    for index, (name, sub, glyph) in enumerate(steps):
        x = 40 + 206 * index
        parts.append(card(x, 100, 184, 100, "store", name, sub, glyph=glyph, dashed=True))
        if index:
            parts.append(wire([(x - 22, 150), (x, 150)], "idle"))
    parts.append(lines(40, 236, ["Step 1 skips quantization: NVIDIA publishes the NVFP4 checkpoint. A model without one adds tensorrt-edgellm-quantize before step 2.",
                                 "Step 3 must run on the Thor: an engine is compiled for the exact GPU that runs it."]))
    save(OUT / "edge-llm-pipeline.svg", 1090, 276,
         "Planned pipeline: NVIDIA's NVFP4 checkpoint from Hugging Face, exported to ONNX with "
         "tensorrt-edgellm-export, built into a TensorRT engine on the Thor with llm_build, served by the "
         "experimental OpenAI-compatible server on port 8081, and measured against llama.cpp with the same prompts.",
         "From checkpoint to measured engine (planned)", "Five steps, each feeding the next.", parts)


# ---- chapter 23: model comparison ----

@figure
def web_switch_decision():
    parts = [
        card(40, 196, 270, 104, "service", "thor-tigress-agent", ["sends the tool definition", "tool_choice: auto", "searches only if a call comes back"]),
        card(400, 100, 250, 96, "model", "Nemotron 3 Nano", ["llama-server :8079", "called the tool 15 of 18"], glyph="nvidia"),
        card(400, 310, 250, 96, "model", "Qwen3.6-35B-A3B", ["Edge-LLM :8081", "called the tool 3 of 18"], glyph="nvidia"),
        card(800, 100, 230, 96, "good", "web_search", ["SearXNG :8888", "6 results, snippets"], glyph="search"),
        card(800, 310, 230, 96, "idle", "answers from memory", ["no tool call this turn", "SearXNG never asked"], dashed=True),
        wire([(310, 236), (355, 236), (355, 148), (400, 148)]), wire([(310, 260), (355, 260), (355, 358), (400, 358)]),
        wire([(650, 148), (800, 148)], label="tool call"), wire([(650, 358), (800, 358)], "idle", label="no tool call"),
        lines(40, 334, ["The same messages and tools", "go to both engines."]),
        text(40, 440, "The engine passes the model's decision through; it does not add or remove tool calls.", "note"),
    ]
    save(OUT / "web-switch-decision.svg", 1070, 462,
         "Both models receive the same request with the web_search tool and tool_choice auto. Nemotron 3 Nano on "
         "llama-server called the tool in 15 of 18 answers and its calls went to SearXNG. Qwen3.6-35B-A3B on "
         "TensorRT Edge-LLM called the tool in 3 of 18 and answered the rest from memory, so SearXNG was never asked.",
         "Web on: the same tool, two decisions", "18 answers per model, thinking off, 2026-10-07.", parts)


@figure
def web_search_bars():
    green, red, track = KINDS["model"][0], KINDS["danger"][0], "#ECE9E0"
    prompts = [("Ownership in Rust", 2, 0), ("Median of a slice", 1, 0), ("Raspberry Pi weekend project", 3, 0),
               ("What is new in CUDA 13?", 3, 3), ("2026 Formula 1 champion", 3, 0), ("RTX 5090 price today", 3, 0)]
    left, unit = 330, 190
    parts = [swatches(60, 104, [(green, "Nemotron 3 Nano: 15 of 18"), (red, "Qwen3.6-35B-A3B: 3 of 18")])]
    parts += [text(left + unit * n, 136, str(n), "note", "middle") for n in range(4)]
    for row, (prompt, nemotron, qwen) in enumerate(prompts):
        y = 150 + 46 * row
        parts.append(text(left - 20, y + 19, prompt, "name", "end"))
        for offset, value, colour in ((0, nemotron, green), (16, qwen, red)):
            parts.append(f'  <rect x="{left}" y="{y + offset}" width="{unit * 3}" height="12" rx="6" fill="{track}"/>')
            if value:
                parts.append(f'  <rect x="{left}" y="{y + offset}" width="{unit * value}" height="12" rx="6" fill="{colour}"/>')
            parts.append(text(left + unit * 3 + 14, y + offset + 10.5, str(value), "pill", fill=colour))
    parts.append(text(60, 444, "Three runs per prompt, Web on, thinking off, default sampling, 2026-10-07.", "note"))
    save(OUT / "web-search-bars.svg", 1000, 466,
         "Answers that called web_search, of three tries each. Nemotron searched for the Rust, Raspberry Pi, CUDA, "
         "Formula 1 and RTX questions and once for the median task, 15 of 18. Qwen searched only for the CUDA "
         "question, 3 of 18.",
         "Answers that called web_search, of three tries each", "Green is Nemotron, red is Qwen. A full bar is three of three.", parts)


# ---- chapter 24: launch note ----

@figure
def vision_canvas():
    uses = [("roleplay", "mask", "#7A4FD0", ["characters who remember", "the scene, with the scene", "kept on the canvas"]),
            ("a therapist", "listen", "#CF4526", ["a calm listener, with", "nothing written down", "on the server"]),
            ("immigration", "passport", "#2F6FDB", ["forms and covering letters", "drafted with you, checked", "against the rules"]),
            ("creative story writing", "pencil", "#B7791F", ["story bibles, drafts and", "revisions standing side", "by side"]),
            ("music generation", "note", "#1F8A70", ["a timeline the model can", "add notes to, and play", "back"]),
            ("vector graphics", "curve", "#5E9400", ["drawing commands it can", "run, and then look at", "what it drew"])]
    parts = [card(40 + 330 * (index % 3), 104 + 140 * (index // 3), 300, 116, "store", name, sub, glyph=glyph,
                  glyph_fill=colour, dashed=True, tile="#FFFFFF", big=True)
             for index, (name, glyph, colour, sub) in enumerate(uses)]
    parts.append(text(40, 410, "Amber and dashed means planned, in this book and on the page.", "note"))
    save(OUT / "vision-canvas.svg", 1040, 432,
         "6 planned uses of the agentic canvas: roleplay with characters who remember the scene; a therapist with "
         "a calm listener and nothing written down on the server; immigration with forms and covering letters "
         "checked against the rules; creative story writing with story bibles, drafts and revisions side by side; "
         "music generation with a timeline the model can add notes to and play back; and vector graphics with "
         "drawing commands it can run and then look at.",
         "The agentic canvas",
         "A sandbox playground where a model works on a canvas, with tools it may reach for. All 6 uses below are planned.", parts)


@figure
def what_is_stored():
    columns = [
        (24, "Your browser", "client", [("conversations", "every message and thought", "browser"),
                                        ("settings", "theme, model, system prompt", "gear"),
                                        ("your key", "kept in local storage", "key")]),
        (360, "The Thor's memory", "idle", [("in memory only", "the reply being written", "chip"),
                                            ("the prompt cache", "so a slot reads it once", "stack"),
                                            ("gone on restart", "read to answer, not saved", "clock")]),
        (696, "The Thor's disk", "store", [("the sealed keyring", "~/.config/thor-chat/keyring", "key"),
                                           ("one record per person", "name, email, status, key", "doc"),
                                           ("the service key", "the servers' own key", "key")]),
    ]
    parts = []
    for x, label, kind, items in columns:
        parts.append(zone(x, 108, 320, 280, label))
        parts += [card(x + 16, 136 + 80 * row, 288, 64, kind, name, sub, glyph=glyph) for row, (name, sub, glyph) in enumerate(items)]
    parts.append(text(520, 420, "Every message sends the whole history; none of it is written down on the Thor.", "note", "middle"))
    save(OUT / "what-is-stored.svg", 1040, 440,
         "3 columns. Your browser holds the conversations, the settings and your key, cleared with site data. The "
         "board's memory holds the reply being written and a small prompt cache, gone on restart. The board's disk "
         "holds the sealed keyring: 1 record per person with a name, an email, a status and a key, and the service "
         "key the servers use between themselves. No message text is written down.",
         "What is stored, and where", "Conversations live in the browser. The Thor's disk holds keys and names, and no messages.", parts)


# ---- chapter 25: keys, and the cryptography under them ----

@figure
def keyring_file():
    def block(x, w, kind, name, sub):
        accent, tint = KINDS[kind]
        centre = x + w / 2
        return "\n".join([
            f'  <rect x="{x}" y="110" width="{w}" height="92" fill="{tint}" stroke="{accent}" stroke-width="1.4"/>',
            text(centre, 140, name, "name", "middle")] + [text(centre, 160 + 17 * row, body, "sub", "middle") for row, body in enumerate(sub)])
    parts = [
        f'  <g filter="url(#lift)">', block(40, 200, "idle", "thor-keyring-1", ["magic, 15 bytes"]),
        block(240, 180, "store", "salt", ["16 random bytes", "stored, not secret"]),
        block(420, 170, "store", "nonce", ["24 bytes", "fresh each write"]),
        block(590, 410, "outside", "the records, sealed", ["ciphertext, then a 16-byte tag"]), "  </g>",
        sheet(40, 250, 440, 132, "store", "EACH PART HAS A JOB", [
            ("", "magic: refuses another version"), ("", "salt: a different key per file"), ("", "nonce: never reused")]),
        sheet(560, 250, 440, 132, "outside", "INSIDE: ONE LINE PER PERSON", [
            ("", '{"name":"Ada Lovelace",'), ("", ' "status":"active","key":"5397…"}')]),
        wire([(795, 202), (795, 250)]),
        text(40, 416, "A wrong passphrase and a changed byte fail the same way.", "note"),
    ]
    save(OUT / "keyring-file.svg", 1040, 436,
         "The keyring file: a 15-byte magic line, a 16-byte random salt, a 24-byte nonce, then the records sealed "
         "as ciphertext with a 16-byte tag. Inside the sealed part is JSON: name, email, status, key and time per person.",
         "The keyring file, byte by byte", "Only the last part is a secret.", parts)


@figure
def passphrase_to_key():
    parts = [
        card(40, 110, 270, 92, "store", "passphrase + salt", ["the salt is 16 random", "bytes, stored in the file"], glyph="key"),
        card(385, 110, 270, 92, "service", "Argon2id", ["19 MiB of memory, 2 passes", "slow on purpose"], glyph="gear"),
        card(730, 110, 270, 92, "good", "a 32-byte key", ["opens the file, then", "wiped from memory"], glyph="key"),
        wire([(310, 156), (385, 156)]), wire([(655, 156), (730, 156)]),
        lines(40, 250, ["A plain hash is fast: a GPU tries billions of guesses a second.",
                        "Argon2id makes each guess hold 19 MiB and take a moment, so guessing slows.",
                        "The same passphrase and salt give the same key, every time."], step=22),
    ]
    save(OUT / "passphrase-to-key.svg", 1040, 330,
         "Argon2id turns a passphrase, a random salt and fixed parameters (19 MiB, 2 passes, 1 lane) into a "
         "32-byte key. A plain hash lets a GPU try billions of guesses a second; Argon2id makes each guess hold "
         "19 MiB and take a moment.",
         "A passphrase becomes a key", "The slow step in the middle is the protection.", parts)


@figure
def aead_seal():
    parts = [
        card(40, 104, 260, 96, "store", "the records", ["plus the magic line", "as associated data"], glyph="doc"),
        card(380, 104, 280, 96, "service", "XChaCha20-Poly1305", ["one key, one nonce", "a fresh nonce per write"], glyph="gear"),
        card(740, 104, 260, 96, "outside", "ciphertext + tag", ["unreadable, and", "any change is caught"], glyph="key"),
        wire([(300, 152), (380, 152)]), wire([(660, 152), (740, 152)]),
        card(40, 250, 470, 84, "idle", "Opening again", ["same key, same nonce, same magic line", "or nothing comes back at all"]),
        card(530, 250, 470, 84, "danger", "One message for two mistakes", ["a wrong passphrase and a changed file", "fail the same way, on purpose"]),
        text(40, 370, "A nonce is not secret, but it must never repeat under one key.", "note"),
    ]
    save(OUT / "aead-seal.svg", 1040, 392,
         "Sealing puts the records and the magic line into XChaCha20-Poly1305 with a 32-byte key and a fresh "
         "24-byte nonce, producing ciphertext and a 16-byte tag. Opening needs the same key, nonce and magic line; "
         "another key, a changed byte or another magic line fails the tag and nothing is returned.",
         "Encrypt and authenticate in one step", "One operation hides the records and proves nobody changed them.", parts)


@figure
def keyring_lifecycle():
    parts = [
        card(60, 130, 200, 80, "idle", "requested", "waiting, no key yet"),
        card(420, 130, 200, 80, "good", "active", "the chat lets it in"),
        card(780, 130, 200, 80, "danger", "revoked", ["key erased,", "record kept"]),
        wire([(260, 158), (420, 158)], label="approve EMAIL"), wire([(620, 158), (780, 158)], label="revoke EMAIL|KEY"),
        wire([(880, 210), (880, 256), (520, 256), (520, 210)], "idle", label="approve again", on=1),
        text(520, 300, "revoke-all: every active key at once", "note", "middle"),
        text(60, 336, "The agent re-reads the file when it changes: no restart.", "note"),
    ]
    save(OUT / "keyring-lifecycle.svg", 1040, 358,
         "A form request creates a requested record. approve mints a key and makes it active. revoke takes a key "
         "away. approve again brings the person back. revoke-all stops every active key with one command. The chat "
         "server reloads the file when it changes, so no restart is needed.",
         "The life of a key", "Three states, and the commands that move a person between them.", parts)


# ---- chapter 26: what the context window is ----

@figure
def context_window():
    held = [("idle", "the system prompt, if any"), ("client", "the conversation so far"), ("client", "anything pasted in"),
            ("good", "the reply being written")]
    parts = [zone(40, 108, 540, 300, "One reply's window")]
    parts += [card(64, 134 + 60 * row, 492, 48, kind, name) for row, (kind, name) in enumerate(held)]
    parts += [
        text(310, 392, "1,048,576 tokens at most, per reply", "note", "middle"),
        card(630, 116, 370, 72, "outside", "No memory between requests", "the browser re-sends everything"),
        card(630, 208, 370, 72, "outside", "One pool of memory", "how many at once is a setting"),
        card(630, 300, 370, 72, "danger", "Full is a wall", "a longer request is refused"),
        text(40, 440, "A token is a piece of a word: about four characters of English.", "note"),
    ]
    save(OUT / "context-window.svg", 1040, 460,
         "One reply's window holds the system prompt, the whole conversation so far, anything pasted in, and the "
         "reply being written, up to 1,048,576 tokens. The model keeps nothing between requests, so the browser "
         "re-sends everything. The windows share one pool of memory: how many run at once is a setting, and a "
         "request longer than its window is refused.",
         "Everything one reply may hold", "The window is the model's whole view of the conversation.", parts)


@figure
def kv_cache():
    blue, amber = KINDS["client"], KINDS["store"]
    parts = [tag(60, 100, "Query: what it wants now", "good"), tag(250, 100, "Key: what each token offers", "client"),
             tag(458, 100, "Value: what each token gives", "store"), text(60, 152, "every earlier token", "note")]
    for index in range(6):
        x = 60 + 100 * index
        parts += [f'  <rect x="{x}" y="164" width="84" height="84" rx="10" fill="#FFFFFF" stroke="{EDGE}" stroke-width="1.3" filter="url(#lift)"/>',
                  text(x + 42, 194, f"t{index + 1}", "name", "middle"),
                  f'  <rect x="{x + 12}" y="210" width="26" height="22" rx="6" fill="{blue[1]}"/>', text(x + 25, 225.5, "K", "pill", "middle", blue[0]),
                  f'  <rect x="{x + 46}" y="210" width="26" height="22" rx="6" fill="{amber[1]}"/>', text(x + 59, 225.5, "V", "pill", "middle", amber[0])]
    parts += [
        card(720, 164, 290, 84, "good", "the token being written", ["its query reads every", "key before it"]),
        wire([(720, 206), (652, 206)]),
        card(60, 290, 584, 90, "service", "The KV cache", ["those keys and values, kept so they are not recomputed",
                                                           "memory grows with the conversation, one entry per token"]),
        card(720, 290, 290, 90, "outside", "the next token", ["the matching values", "decide the word"]),
        wire([(865, 248), (865, 290)]),
        lines(60, 416, ["This model is a hybrid: only 6 of its 52 layers use attention this way.",
                        "The rest keep a fixed state, which is why a million tokens fit on the Thor."], step=20),
    ]
    save(OUT / "kv-cache.svg", 1050, 456,
         "Every earlier token has a key and a value. The query of the token being written is compared with every "
         "key before it, and the matching values decide the next token. The keys and values kept are the KV cache, "
         "so memory grows with the conversation. This model is a hybrid: only 6 of its 52 layers use attention this way.",
         "What KV means: a key and a value per token", "Each token leaves two things behind for the tokens after it.", parts)


# ---- chapter 27: sharing is caring ----

@figure
def sharing_is_caring():
    parts = [
        card(40, 130, 210, 72, "outside", "Friend's laptop", "Tailscale on", glyph="tailscale"),
        card(40, 300, 210, 72, "client", "Your machines", "as today", glyph="tailscale"),
        zone(330, 108, 690, 296, "Jetson AGX Thor"),
        card(360, 118, 280, 96, "outside", "friend", ["sudo · video · render", "their home, their key"], glyph="terminal"),
        card(360, 294, 280, 84, "client", "arpanpathak", "your home, your keys", glyph="terminal"),
        card(730, 200, 260, 100, "model", "NVIDIA Thor", ["one GPU,", "time-shared"], glyph="nvidia"),
        wire([(250, 166), (360, 166)], label="tailnet"), wire([(250, 336), (360, 336)], label="tailnet"),
        wire([(640, 166), (685, 166), (685, 238), (730, 238)]), wire([(640, 336), (685, 336), (685, 262), (730, 262)]),
        lines(40, 440, ["Two logins on one machine, no port open at home.",
                        "You share the Thor; your other machines stay yours, and theirs stay theirs."]),
    ]
    save(OUT / "sharing-is-caring.svg", 1044, 480,
         "Your friend's laptop and your machines reach the Thor over the tailnet. The Thor holds two logins, "
         "friend and arpanpathak, and one GPU the two of them share.",
         "Two people, one Thor", "Each person has a login. The GPU is the only thing they share.", parts)


@figure
def friend_own_node():
    parts = [
        zone(270, 116, 250, 150, "Their tailnet"), zone(690, 116, 330, 150, "Your tailnet, unchanged"),
        card(40, 154, 190, 72, "client", "visitor", "app.example.com", glyph="browser"),
        card(290, 154, 210, 72, "outside", "their node", "Funnel + proxy", glyph="tailscale"),
        card(710, 154, 290, 72, "model", "Jetson AGX Thor", "their app · 127.0.0.1:9100", glyph="nvidia"),
        wire([(230, 190), (290, 190)]), wire([(500, 190), (710, 190)], label="shared node"),
        text(40, 310, "Their app uses the GPU here. The name on the link is theirs, and your chat's address stays as it is.", "note"),
    ]
    save(OUT / "friend-own-node.svg", 1044, 336,
         "A visitor reaches app.example.com, which lands on a Tailscale node of their own running Funnel and a "
         "proxy. That node reaches the Thor over the tailnet, where their app listens on 127.0.0.1:9100. The Thor's "
         "own name does not change.",
         "Their name in front, the Thor behind it", "The visitor never sees the Thor's address.", parts)


@figure
def friend_funnel_paths():
    rows = [
        (("client", "anyone on the internet", "https://…ts.net/"), ("outside", "Funnel :443 · /", "your chat"),
         ("service", "thor-tigress-agent", "127.0.0.1:8080"), "flow"),
        (("client", "their domain", "forwarding page · 302"), ("outside", "Funnel :443 · /friend-app", "their app, public"),
         ("service", "their app", "127.0.0.1:9100"), "flow"),
        (("client", "their devices, on the tailnet", "no public hop"), ("idle", "serve :8443", "tailnet only"),
         ("service", "the same app", "stays private"), "idle"),
    ]
    parts = [zone(380, 112, 660, 352, "thor-tigress.cat-crocodile.ts.net")]
    for index, (who, way, app, kind) in enumerate(rows):
        y = 142 + 106 * index
        parts += [card(40, y, 280, 72, *who), card(400, y, 280, 72, *way), card(740, y, 280, 72, *app),
                  wire([(320, y + 36), (400, y + 36)], kind), wire([(680, y + 36), (740, y + 36)], kind)]
    parts.append(text(40, 490, "One name, one certificate. The path or the port picks which app answers.", "note"))
    save(OUT / "friend-funnel-paths.svg", 1064, 510,
         "Three ways into the Thor. Anyone on the internet reaches your chat at the root path. Their domain goes "
         "through a forwarding page to the friend-app path. Their own devices reach the same app privately on port "
         "8443 with tailscale serve.",
         "Their app, their domain, one Funnel", "Three ways in, each ending at a port on 127.0.0.1.", parts)


# ---- introduction ----

@figure
def project_map():
    parts = [
        zone(24, 108, 500, 372, "The tigress · the platform"),
        card(48, 136, 452, 60, "service", "spark", "scores answers: slop phrases and the five Rust rules", glyph="rust"),
        card(48, 206, 452, 60, "service", "thor-hammer-trainer", "builds the training set from books, docs and chats", glyph="rust"),
        card(48, 276, 452, 60, "service", "reinforcer", "the review page for the training set", glyph="rust"),
        card(48, 346, 452, 60, "service", "lasso", "conversations from books, questions by a teacher model", glyph="rust"),
        card(48, 416, 452, 50, "store", "Fine-tuning, and scoring before and after", dashed=True),

        zone(540, 108, 500, 372, "The cub · on the Jetson Thor"),
        card(564, 136, 452, 60, "client", "The chat page", "model picker, thinking panel, web search", glyph="browser"),
        card(564, 206, 452, 60, "service", "thor-tigress-agent", "the page, the keys, the tools and the API", glyph="rust"),
        card(564, 276, 452, 60, "service", "thor-tigress-keyring", "who may chat: one sealed record and key per person", glyph="key"),
        card(564, 346, 452, 60, "model", "The model servers", "llama.cpp and TensorRT Edge-LLM", glyph="nvidia"),
        card(564, 416, 452, 50, "idle", "thor-tigress-serve: list, download, load, unload"),
        legend(48, 510, [("service", "built"), ("model", "running on the Thor")]),
        f'  <rect x="330" y="500" width="14" height="14" rx="4" fill="{KINDS["store"][1]}" stroke="{KINDS["store"][0]}" stroke-width="1.3" stroke-dasharray="3 2"/>',
        text(351, 511.5, "planned", "note"),
    ]
    save(OUT / "project-map.svg", 1064, 534,
         "Two groups. The tigress, the platform: spark scores answers, thor-hammer-trainer builds the training set, "
         "reinforcer is the review page, lasso makes conversations from books, and fine-tuning with before and "
         "after scoring is planned. The cub, on the Jetson Thor: the chat page, thor-tigress-agent serving the "
         "page, keys, tools and API, thor-tigress-keyring holding who may chat, the model servers llama.cpp and "
         "TensorRT Edge-LLM, and thor-tigress-serve managing the models.",
         "The parts of the project", "The tigress makes and measures the data. The cub is what people talk to.", parts)


# ---- chapter 2: the checker ----

@figure
def spark_scoring():
    parts = [
        sheet(40, 124, 250, 186, "client", "ONE ANSWER", [
            ("", "Here is the code. It"), ("", "never uses unwrap."), ("dim", "```rust"), ("trap", "let x = v.unwrap();"), ("dim", "```")],
            ["Prose, then a Rust block."]),
        card(340, 172, 190, 88, "service", "answer.rs", ["splits the prose", "from the Rust blocks"]),
        card(590, 104, 230, 88, "service", "slop.rs", ["reads the prose:", "stock slop phrases,", "code blanked out first"]),
        card(590, 216, 230, 88, "service", "rules.rs", ["reads each Rust block:", "five rules, each pass,", "fail or not applicable"]),
        card(590, 336, 230, 72, "idle", "comments.rs", ["a lexer for comments,", "which syn throws away"]),
        card(870, 150, 180, 104, "danger", "claims.rs", ["prose that says the", "rules are followed", "when one failed"]),
        cylinder(870, 318, 180, 96, "store", "report.rs", ["adds answers up", "into rates"]),
        wire([(290, 216), (340, 216)]),
        wire([(530, 204), (560, 204), (560, 148), (590, 148)]), wire([(530, 228), (560, 228), (560, 260), (590, 260)]),
        wire([(705, 336), (705, 304)], "idle"),
        wire([(820, 148), (845, 148), (845, 190), (870, 190)]), wire([(820, 260), (845, 260), (845, 214), (870, 214)]),
        wire([(960, 254), (960, 318)]),
        lines(40, 446, ["In the answer on the left, rule 1 fails on the red line, and the first two lines are a false claim.",
                        "spark reads code. It does not compile it or run its tests."]),
    ]
    save(OUT / "spark-scoring.svg", 1080, 484,
         "One answer, prose and a Rust block, goes to answer.rs, which splits them. slop.rs reads the prose for "
         "stock slop phrases with code blanked out. rules.rs reads each Rust block against the five rules, giving "
         "pass, fail or not applicable, with comments.rs finding the comments. claims.rs finds prose that says the "
         "rules are followed when one failed. report.rs adds the answers up into rates.",
         "How spark scores one answer", "Three checks on one answer: its prose, its code, and whether the prose is honest about the code.", parts)


# ---- chapter 7: access and syncing ----

@figure
def thor_sync_flow():
    parts = [
        zone(24, 108, 480, 300, "Your machine"), zone(700, 108, 340, 300, "Jetson Thor"),
        card(48, 140, 260, 64, "store", "~/Projects/my-project", "git history included", glyph="doc"),
        card(48, 250, 260, 80, "service", "thor-sync", ["watches the folders", "on its list"], glyph="gnubash"),
        cylinder(340, 240, 150, 100, "store", "~/.config/", ["thor-sync/:", "host, folders,", "exclude, include"]),
        card(724, 250, 292, 80, "store", "~/Projects/my-project", ["the same place under", "the Thor's home folder"], glyph="doc"),
        wire([(178, 204), (178, 250)]), wire([(340, 290), (308, 290)], "idle"),
        wire([(178, 330), (178, 372), (870, 372), (870, 330)], label="rsync over ssh thor · one way", on=1),
        lines(48, 438, ["Skipped: what .gitignore skips, and the exclude list. The include list copies ignored paths anyway.",
                        "Deleting a file on your machine never deletes it on the Thor."]),
    ]
    save(OUT / "thor-sync-flow.svg", 1064, 478,
         "On your machine, thor-sync watches the folders on its list, reads its host, folder, exclude and include "
         "files from ~/.config/thor-sync, and copies each folder with rsync over ssh, one way, to the same place "
         "under the Thor's home folder. It skips what .gitignore and the exclude list skip, and never deletes on the Thor.",
         "What thor-sync copies, and where it lands", "One direction only: from your machine to the Thor.", parts)


@figure
def coffee_shop_ssh():
    friends = ["long", "beard", "curly", "glasses", "bun", "cap"]
    parts = [
        zone(24, 108, 340, 380, "A coffee shop, anywhere"), zone(700, 108, 340, 380, "At home · Jetson AGX Thor"),
        *[person(54 + 100 * (seat % 3), 132 + 92 * (seat // 3), look, 80) for seat, look in enumerate(friends)],
        text(194, 326, "you, and the friends you gave a login", "note", "middle"),
        card(48, 346, 292, 84, "client", "A laptop each", ["Tailscale on, signed in", "ssh thor"], glyph="laptop"),
        icon("cup", 48, 446, 22, KINDS["store"][0]), text(78, 462, "café Wi-Fi: nothing to set up on it", "note"),
        card(412, 346, 240, 84, "outside", "Tailscale", ["WireGuard, end to end", "direct, or by a relay"], glyph="tailscale"),
        card(724, 140, 292, 72, "service", "Tailscale SSH", "checks who you are", glyph="key", pill="tailnet only"),
        card(724, 260, 292, 72, "client", "Your shell", "nvidia-smi · tegrastats", glyph="terminal"),
        card(724, 380, 292, 72, "model", "NVIDIA Thor", "one GPU, time-shared", glyph="nvidia"),
        wire([(340, 388), (412, 388)]),
        wire([(652, 388), (676, 388), (676, 176), (724, 176)]),
        wire([(870, 212), (870, 260)]), wire([(870, 332), (870, 380)]),
        lines(24, 522, ["No port is opened on the home router, and the café's network sees only encrypted packets.",
                        "The public Funnel address carries the chat page. A shell needs a device on the tailnet."]),
    ]
    save(OUT / "coffee-shop-ssh.svg", 1064, 562,
         "Six people in a coffee shop, women and men, each with a laptop that has Tailscale on and signed in. Their ssh "
         "connections go through Tailscale, as WireGuard from end to end, either direct or by a relay, to the "
         "Jetson AGX Thor at home. There, Tailscale SSH checks who each person is, gives them their shell, and "
         "the shell reaches the one GPU they share.",
         "A shell on the Thor, from a coffee shop", "The laptop and the Thor are on one private network, wherever the laptop is.", parts)


@figure
def ssh_alias():
    meanings = [("client", "Where to go", "HostName and User: the address, the login", "search"),
                ("store", "Who you are", "IdentityFile: the key that proves it", "key"),
                ("service", "Stay connected", "ControlMaster: later commands reuse it", "swap")]
    parts = [
        card(24, 176, 220, 72, "client", "ssh thor", "what you type", glyph="terminal"),
        sheet(290, 108, 330, 210, "store", "~/.SSH/CONFIG, READ BY SSH", [
            ("strong", "Host thor"), ("", "    HostName <thor-ip>"), ("", "    User <user>"),
            ("", "    IdentityFile ~/.ssh/id_ed25519"), ("", "    ControlMaster auto"), ("", "    ControlPersist 10m")]),
        *[card(680, 108 + 74 * row, 360, 62, kind, name, sub, glyph=glyph) for row, (kind, name, sub, glyph) in enumerate(meanings)],
        wire([(244, 212), (290, 212)]),
        *[wire([(620, 139 + 74 * row), (680, 139 + 74 * row)]) for row in range(3)],
        lines(24, 352, ["ssh looks the name up in the file and fills in the rest. scp, rsync and thor-sync read the same file."]),
    ]
    save(OUT / "ssh-alias.svg", 1064, 378,
         "You type ssh thor. ssh looks up Host thor in the file ~/.ssh/config and finds five settings. HostName and "
         "User say where to go and which login to use. IdentityFile names the key that proves who you are. "
         "ControlMaster and ControlPersist keep one connection open so later commands reuse it.",
         "What the alias stands for", "One short name in place of an address, a login and a key.", parts)


@figure
def mac_setup():
    steps = [
        ("client", "1 · Join the tailnet", "tailscale",
         ["Install the Tailscale app and sign in with the account", "the Thor is on. The Thor then shows up as arpanpathak."]),
        ("store", "2 · Have a key on the Thor", "key",
         ["ls ~/.ssh/*.pub. None? Run ssh-keygen, then ssh-copy-id,", "at home, where the Thor still takes your password."]),
        ("service", "3 · Write one alias", "terminal",
         ["Host thor in ~/.ssh/config, with HostName", "arpanpathak.taildb9a39.ts.net"]),
        ("good", "4 · Try it on a phone hotspot", "laptop",
         ["Wi-Fi off, hotspot on, ssh thor.", "If that gives a prompt, the coffee shop will too."]),
    ]
    parts = [card(24 + 520 * (index % 2), 108 + 124 * (index // 2), 496, 96, kind, name, sub, glyph=glyph)
             for index, (kind, name, glyph, sub) in enumerate(steps)]
    parts += [
        wire([(520, 156), (544, 156)]),
        wire([(792, 204), (792, 218), (272, 218), (272, 232)]),
        wire([(520, 280), (544, 280)]),
        lines(24, 362, ["All four are done on the Mac, at home. Nothing is changed on the Thor except one line in authorized_keys."]),
    ]
    save(OUT / "mac-setup.svg", 1064, 388,
         "Four steps on the Mac, done at home. One: install the Tailscale app and sign in with the account the Thor "
         "is on. Two: make sure the Mac has an ssh key on the Thor, creating one with ssh-keygen and installing it "
         "with ssh-copy-id if not. Three: write one alias, Host thor, pointing at the Thor's tailnet name. Four: "
         "turn Wi-Fi off, use a phone hotspot, and run ssh thor to prove it works away from home.",
         "Setting up a Mac, once", "Four steps at home, then the same command works anywhere.", parts)


@figure
def coffee_shop_safety():
    hops = [("client", "Your Mac", "encrypts every packet", "laptop"), ("idle", "Café Wi-Fi", "sees scrambled packets", "cup"),
            ("outside", "Tailscale relay", "passes them, can't read", "tailscale"), ("model", "The Thor", "no port open at home", "nvidia")]
    risks = [
        ("Your Tailscale account", "key", ["Whoever signs in to it can add a device.", "Turn on two-factor sign-in."]),
        ("The Mac itself", "laptop", ["Signed in means trusted. Lock the screen, use FileVault,", "and remove a lost Mac on the admin page."]),
        ("A key on a pendrive", "lock", ["Give the key a passphrase. Without one, whoever", "finds the pendrive holds the key."]),
        ("The person behind you", "eye", ["No tunnel hides your screen.", "Sit with your back to a wall."]),
    ]
    parts = [card(24 + 262 * index, 112, 230, 80, kind, name, sub, glyph=glyph) for index, (kind, name, sub, glyph) in enumerate(hops)]
    parts += [wire([(254 + 262 * gap, 152), (286 + 262 * gap, 152)]) for gap in range(3)]
    parts.append(zone(24, 246, 1016, 220, "Where the risk really is"))
    parts += [card(40 + 500 * (index % 2), 276 + 92 * (index // 2), 484, 76, "danger", name, sub, glyph=glyph, dashed=True, tile="#FFFFFF")
              for index, (name, glyph, sub) in enumerate(risks)]
    parts.append(lines(24, 500, ["The network in between is the part you do not have to trust. The account, the laptop and the key are."]))
    save(OUT / "coffee-shop-safety.svg", 1064, 526,
         "Along the top, the path of a packet: your Mac encrypts it, the café Wi-Fi sees only scrambled packets, a "
         "Tailscale relay passes them on without being able to read them, and the Thor at home has no port open. "
         "Below, the four real risks: your Tailscale account, which needs two-factor sign-in; the Mac itself, which "
         "needs a locked screen and disk encryption; a key on a pendrive, which needs a passphrase; and the person "
         "behind you, who can read your screen.",
         "Is it safe from a coffee shop?", "Yes for the network. The risks are the things you carry.", parts)


@figure
def tunnel_uses():
    parts = [
        sheet(24, 108, 328, 190, "service", "1 · A SHELL", [
            ("", "ssh thor"), ("", "nvidia-smi"), ("", "tegrastats --interval 2000"), ("", "free -h")],
            ["Look at the GPU and the memory."]),
        sheet(368, 108, 328, 190, "client", "2 · A PORT, BROUGHT TO YOU", [
            ("", "ssh -N -L 8080:127.0.0.1:8080 thor"), ("dim", "# then, in the browser:"), ("", "http://localhost:8080")],
            ["Servers on the Thor stay on localhost."]),
        sheet(712, 108, 328, 190, "model", "3 · A JOB THAT STAYS", [
            ("", "systemd-run --user --unit job \\"), ("", "    ./train.sh"), ("", "journalctl --user -u job -f")],
            ["Close the lid; it keeps running."]),
        lines(24, 330, ["All three ride the same ssh connection. Nothing else is installed on the laptop."]),
    ]
    save(OUT / "tunnel-uses.svg", 1064, 356,
         "Three things to do over the ssh connection. One: a shell, to look at the GPU with nvidia-smi, tegrastats "
         "and free. Two: a port forwarded to the laptop, so a server that listens only on the Thor's localhost "
         "opens in the laptop's browser. Three: a job started with systemd-run, which keeps running after the "
         "laptop is closed and is followed with journalctl.",
         "Three things the connection is for", "Once ssh thor works from outside, these work too.", parts)


# ---- chapter 14: writing the papers (planned) ----

@figure
def reports_evidence():
    rows = [("Stage 1 (done)", "and a few hundred slop flags", "1. Dataset and curation", "arXiv report; data workshop"),
            ("Stage 2", "and the evaluation design", "2. Fine-tuning results", "arXiv; workshop paper"),
            ("Stage 2 measurements", "memory, throughput, step time", "3. Jetson AGX Thor", "systems workshop; blog")]
    parts = [card(400, 116, 240, 296, "danger", "Claims and evidence", [
        "A claim goes into a", "report only if its", "evidence row is met:", "", "a paired test,", "an interval,",
        "frozen held-out items,", "a second rater."])]
    for index, (need, detail, report, venue) in enumerate(rows):
        y = 116 + 110 * index
        parts += [card(40, y, 290, 76, "idle" if index else "service", need, detail),
                  card(710, y, 300, 76, "store", report, venue, dashed=True),
                  wire([(330, y + 38), (400, y + 38)], "idle"), wire([(640, y + 38), (710, y + 38)], "idle")]
    parts.append(text(40, 444, "Results that did not improve are reported with the same detail as those that did.", "note"))
    save(OUT / "reports-evidence.svg", 1050, 466,
         "Three planned reports, each needing something first: Stage 1 and slop flags for the dataset report, Stage "
         "2 and the evaluation for the fine-tuning report, Stage 2 measurements for the Jetson report. Between "
         "what exists and each report stands one gate: a claim goes in only if its evidence row is met.",
         "What each report needs, and the gate every claim passes", "Left: what has to exist first. Right: the report it allows.", parts)


# ---- chapter 15: workspace agent (planned, paused) ----

@figure
def workspace_sandbox():
    plan = lambda x, y, w, h, name, sub, glyph=None: card(x, y, w, h, "store", name, sub, glyph=glyph, dashed=True)
    parts = [
        plan(40, 206, 200, 80, "Browser", ["a random workspace id", "with every request"]),
        plan(300, 196, 230, 100, "thor-tigress-agent", ["maps the id to a folder", "and runs the tool loop"]),
        zone(590, 108, 450, 200, "A container per workspace · no network"),
        plan(610, 134, 210, 72, "run_command", ["non-root, with CPU,", "memory and time limits"]),
        plan(610, 222, 210, 64, "list, read, write", "files in that folder"),
        cylinder(856, 150, 164, 104, "store", "the folder", ["the only thing", "mounted"]),
        plan(610, 350, 210, 80, "web_search, fetch", ["run by the agent,", "not in the container"]),
        plan(876, 350, 164, 80, "The internet", ["public addresses", "only"]),
        wire([(240, 246), (300, 246)], "idle"),
        wire([(530, 234), (560, 234), (560, 170), (610, 170)], "idle"),
        wire([(530, 258), (560, 258), (560, 390), (610, 390)], "idle"),
        wire([(820, 170), (856, 170)], "idle"), wire([(820, 254), (838, 254), (838, 222), (856, 222)], "idle"),
        wire([(820, 390), (876, 390)], "idle"),
        text(40, 462, "Everything here is planned, and the work is paused. Page fetching is the one part already built, for the chat.", "note"),
    ]
    save(OUT / "workspace-sandbox.svg", 1064, 484,
         "Planned design. The browser sends a random workspace id with every request. thor-tigress-agent maps the "
         "id to a folder and runs the tool loop. Commands and file tools run in a container per workspace, as a "
         "non-root user with CPU, memory and time limits, no network, and only that workspace's folder mounted. "
         "Web search and page fetching are run by the agent outside the container and reach public addresses only.",
         "The workspace agent's sandbox (planned)", "Code the model runs is boxed in. Only the agent itself may reach the network.", parts)


# ---- chapter 17: bring your own domain ----

@figure
def domain_hops():
    parts = [
        card(40, 200, 190, 72, "client", "Browser", "voltforge.tech/…", glyph="browser"),
        card(300, 110, 270, 72, "outside", "Namecheap nameservers", "A records: GitHub's addresses"),
        card(650, 104, 270, 84, "outside", "GitHub Pages", ["the forwarding page", "Let's Encrypt certificate"], glyph="github"),
        card(300, 300, 270, 84, "outside", "Tailscale Funnel", ["answers only for its own", ".ts.net name"], glyph="tailscale"),
        card(650, 300, 270, 84, "model", "Jetson AGX Thor", ["thor-tigress-agent", "127.0.0.1:8080"], glyph="nvidia"),
        wire([(230, 224), (265, 224), (265, 146), (300, 146)]),
        wire([(230, 236), (610, 236), (610, 160), (650, 160)], label="2 · connect, and get the page", on=0),
        wire([(230, 248), (265, 248), (265, 342), (300, 342)]),
        wire([(570, 342), (650, 342)]),
        tag(150, 160, "1 · lookup", "client"), tag(160, 296, "3 · jump", "client"),
        lines(40, 420, ["A domain cannot point at Funnel directly: Funnel holds a certificate only for its own name.",
                        "So GitHub Pages answers for voltforge.tech, with its own certificate, and sends the browser on."]),
    ]
    save(OUT / "domain-hops.svg", 960, 462,
         "Three steps. One, the browser asks Namecheap's nameservers, whose A records give GitHub's addresses. Two, "
         "it connects to GitHub Pages, which holds a Let's Encrypt certificate for voltforge.tech and returns the "
         "forwarding page. Three, the page sends the browser to the .ts.net name, where Tailscale Funnel answers "
         "and passes the request to thor-tigress-agent on the Thor at 127.0.0.1:8080.",
         "From the short address to the chat", "Three steps: a lookup, a page, and a jump.", parts)


# ---- chapter 18: operations ----

@figure
def boot_order():
    units = [("thor-chat", ["llama-server"]), ("thor-edge-llm", ["one unit per", "Edge-LLM model"]), ("searxng", ["web search"]),
             ("thor-tigress-agent", ["starts after the", "model servers"])]
    parts = [
        card(40, 120, 150, 64, "idle", "Power on"),
        card(250, 112, 280, 80, "outside", "tailscaled", ["system service: joins the", "tailnet, restores Funnel"], glyph="tailscale"),
        card(600, 112, 280, 80, "service", "the user's systemd", ["starts with nobody logged", "in, because of linger"], glyph="linux"),
        wire([(190, 152), (250, 152)], label=None), wire([(530, 152), (600, 152)]),
        tag(196, 124, "1", "idle"), tag(548, 124, "2", "idle"),
    ]
    for index, (name, sub) in enumerate(units):
        x = 40 + 250 * index
        parts += [card(x, 292, 226, 84, "service", name, sub),
                  wire([(740, 192), (740, 244), (x + 113, 244), (x + 113, 292)])]
    parts += [
        text(40, 412, "Each service restarts itself 5 seconds after a crash (Restart=on-failure).", "note"),
        text(40, 432, "Funnel is restored in step 1, so the public address answers as soon as the agent is up.", "note"),
    ]
    save(OUT / "boot-order.svg", 1040, 456,
         "At power on, tailscaled starts as a system service, joins the tailnet and restores Funnel. Because of "
         "linger, the user's systemd then starts with nobody logged in and brings up thor-chat, one thor-edge-llm "
         "unit per Edge-LLM model, searxng, and thor-tigress-agent after the model servers. Each service restarts "
         "itself five seconds after a crash.",
         "What starts at boot, and in what order", "Nobody has to log in for the chat to come back.", parts)


wanted = sys.argv[2:] or list(FIGURES)
for name in wanted:
    FIGURES[name]()
print("wrote", len(wanted), "figures to", OUT)
