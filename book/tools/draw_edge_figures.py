"""Draws the four figures of the chapter "Coding tools on TensorRT Edge-LLM".

    python3 book/tools/draw_edge_figures.py book/src/figures
"""
import sys
from pathlib import Path

from figkit import INK, KINDS, MUTED, PAPER, card, cylinder, icon, must_fit, save, sheet, text, wire, zone

OUT = Path(sys.argv[1])


def note(x, y, lines, kind):
    """Lines of small text in a kind's colour, 17 px apart."""
    return "\n".join(text(x, y + 17 * row, body, "note", fill=KINDS[kind][0]) for row, body in enumerate(lines))


def serving():
    client = lambda y: card(48, y, 176, 64, "client", "Coding tool", "or the chat page", glyph="terminal")
    agent = lambda y, sub: card(264, y, 260, 72, "service", "thor-tigress-agent", [":8080", sub], glyph="swap")
    engine = lambda y, name, sub: card(568, y, 262, 56, "model", name, sub, glyph="nvidia")

    before = [
        zone(24, 100, 992, 220, "Before · 2026-10-10"),
        client(186), agent(182, "exact name, or the default"),
        card(568, 122, 262, 56, "danger", "llama-server router", ":8079 · the default", glyph="stack", dashed=True),
        engine(190, "Qwen3.6 35B", "Edge-LLM · :8081"),
        engine(258, "Nemotron 3.5 Lightning", "Edge-LLM · :8082"),
        cylinder(858, 116, 150, 68, "danger", "Nemotron 3 Nano", "GGUF on disk"),
        wire([(224, 218), (264, 218)]),
        wire([(524, 206), (546, 206), (546, 150), (568, 150)], "danger"),
        wire([(524, 218), (568, 218)]),
        wire([(524, 230), (546, 230), (546, 286), (568, 286)]),
        wire([(830, 150), (858, 150)], "danger"),
        note(858, 214, ["A name no engine has", "went to the router.", "A name it knew put", "35 GB back in memory."], "danger"),
    ]
    after = [
        zone(24, 350, 992, 290, "After · 2026-10-11"),
        client(444), agent(440, "asks who has the model"),
        engine(380, "Qwen3.6 35B", "Edge-LLM · :8081 · first"),
        engine(448, "Nemotron 3.5 Lightning", "Edge-LLM · :8082"),
        card(568, 516, 262, 56, "idle", "llama-server router", ":8079 · autoload off", glyph="stack", dashed=True),
        wire([(224, 476), (264, 476)]),
        wire([(524, 464), (546, 464), (546, 408), (568, 408)]),
        wire([(524, 476), (568, 476)]),
        wire([(524, 488), (546, 488), (546, 544), (568, 544)], "idle"),
        note(858, 404, ["its own name, or a", "name nobody has"], "good"),
        note(858, 480, ["its own name only"], "good"),
        note(858, 527, ["what it has loaded.", "It loads at startup or", "on “load”, never", "on a request."], "idle"),
        text(48, 618, "AGENT_DEFAULT=edge puts the engines first. Without it llama-server is first, and is the default.", "note"),
    ]
    save(OUT / "edge-serving-before-after.svg", 1040, 664,
         "Before: thor-tigress-agent sent a request naming an Edge-LLM model to that engine and every other name to "
         "the llama-server router on port 8079, which loaded Nemotron 3 Nano from disk when a request named it. "
         "After: the agent asks which server has the model. Qwen3.6 35B on 8081 answers for its own name and for "
         "a name nobody has, Nemotron 3.5 Lightning on 8082 for its own name, and the llama-server router on 8079 "
         "for the ids and aliases of the models it has loaded. The router runs with autoload off, so it loads a "
         "model at startup or on the load command and never because a request named one.",
         "Which server answers a request", "The same three servers, and a different rule for choosing between them.",
         before + after)


def stage(x, y, number, name, before, after, cost):
    """One stage of the request path: what comes in, what goes out, and what the engine said without it."""
    accent, tint = KINDS["service"]
    red, blush = KINDS["danger"]
    for body in (before, after):
        must_fit(body, 168, 12, mono=True)
    must_fit(cost, 168)
    return "\n".join([
        f'  <rect x="{x}" y="{y}" width="200" height="132" rx="12" fill="#FFFFFF" stroke="#D5DBE3" stroke-width="1.3" filter="url(#lift)"/>',
        f'  <path d="M{x + 0.7},{y + 100} H{x + 199.3} V{y + 120} A11.3,11.3 0 0 1 {x + 188},{y + 131.3} H{x + 12}'
        f' A11.3,11.3 0 0 1 {x + 0.7},{y + 120} Z" fill="{blush}"/>',
        f'  <circle cx="{x + 28}" cy="{y + 28}" r="13" fill="{tint}"/>',
        text(x + 28, y + 32.8, str(number), "name", "middle", accent),
        text(x + 50, y + 33, name, "name"),
        text(x + 16, y + 64, before, "mono dim"),
        text(x + 16, y + 84, after, "mono"),
        text(x + 16, y + 120, cost, "note", fill=red),
    ])


def fit():
    stages = [
        (40, 96, 1, "Route", 'model: "gpt-4o"', "→ the first server", "without it: 404"),
        (280, 96, 2, "Name", 'model: "gpt-4o"', '→ "Qwen3.6-35B-…"', "without it: 404"),
        (520, 96, 3, "Fields", "user, store, metadata", "→ dropped", "without it: 400"),
        (760, 96, 4, "System", "system, user, system", "→ one system, first", "without it: 500 from Qwen"),
        (40, 300, 5, "Arguments", '"{\\"path\\":\\"a.rs\\"}"', '→ {"path": "a.rs"}', "without it: steps repeat"),
        (280, 300, 6, "Mode", "brings its own tools", "→ relayed, untouched", "without it: calls dropped"),
    ]
    parts = [stage(*one) for one in stages] + [
        card(520, 334, 200, 64, "model", "Edge-LLM engine", "that has the model", glyph="nvidia"),
        card(760, 334, 200, 64, "client", "The answer", ["copied back as it", "comes"], glyph="terminal"),
        wire([(240, 162), (280, 162)]), wire([(480, 162), (520, 162)]), wire([(720, 162), (760, 162)]),
        wire([(860, 228), (860, 264), (140, 264), (140, 300)]),
        wire([(240, 366), (280, 366)]), wire([(480, 366), (520, 366)]), wire([(720, 366), (760, 366)]),
        text(40, 466, "Stages 1 and 2: config.rs and body.rs.   Stages 3 to 5: chat/shape.rs.   Stage 6: chat/mod.rs.", "note"),
        text(40, 486, "A request for llama-server takes stages 1 and 6 only: it is passed on as it came.", "note"),
    ]
    save(OUT / "edge-request-fit.svg", 1000, 510,
         "Six stages a chat request passes in thor-tigress-agent before a TensorRT Edge-LLM engine sees it: route by "
         "model name, name the engine's own model, keep only the fields the engine knows, join system messages into "
         "one at the top, turn earlier tool-call arguments from JSON text into objects, and relay untouched when the "
         "request brings its own tools. Under each stage is what the engine answered without it: 404, 404, 400, "
         "500, repeated steps, and dropped calls. A request for llama-server takes only the first and last stage.",
         "What the agent does to a chat request before an Edge-LLM engine sees it",
         "Grey is what arrives, black is what leaves. The red line is what the engine answered without the stage.",
         parts)


def arguments():
    call = lambda value: [
        ("", '{ "role": "assistant", "tool_calls": [ {'),
        ("", '    "id": "call_1", "type": "function",'),
        ("", '    "function": {'),
        ("", '      "name": "write_file",'),
        ("", f'      "arguments": {value}'),
        ("", "    } } ] }"),
    ]
    parts = [
        sheet(40, 96, 440, 200, "client", "WHAT A CODING TOOL SENDS BACK", call('"{\\"path\\": \\"fizz.py\\", …}"'),
              ["Arguments as JSON text: the OpenAI format."]),
        wire([(480, 196), (560, 196)], "danger"),
        sheet(560, 96, 400, 200, "danger", "WHAT THE MODEL READS · 471 PROMPT TOKENS", [
            ("", "<tool_call>"), ("", "<function=write_file>"), ("dim", "    (no parameters)"),
            ("", "</function>"), ("", "</tool_call>"),
        ], ["Qwen: “I called write_file without providing", "the required path and content parameters”"]),

        sheet(40, 330, 440, 200, "service", "WHAT THE AGENT NOW SENDS", call('{ "path": "fizz.py", … }'),
              ["The same call. One value changed type."]),
        wire([(480, 430), (560, 430)]),
        sheet(560, 330, 400, 200, "good", "WHAT THE MODEL READS · 518 PROMPT TOKENS", [
            ("", "<tool_call>"), ("", "<function=write_file>"), ("", "<parameter=path>fizz.py</parameter>"),
            ("", "<parameter=content>…</parameter>"), ("", "</function></tool_call>"),
        ], ["Qwen: “The test ran successfully with exit", "code 0.”"]),
        text(40, 560, "The text on the right is inferred from the token counts and the replies. The engine does not return the prompt it built.", "note"),
    ]
    save(OUT / "edge-arguments-rendering.svg", 1000, 582,
         "An assistant message with a tool call whose arguments are JSON text is rendered by the engine's chat "
         "template as a write_file call with no parameters, 471 prompt tokens, and Qwen says it called write_file "
         "without the required path and content. The same message with the arguments as an object is rendered "
         "with both parameters, 518 prompt tokens, and Qwen reads the result correctly.",
         "One earlier tool call, as a string and as an object",
         "Measured on Nemotron 3.5 Lightning for the token counts; the words are Qwen's.", parts)


def timeline():
    left, right, start, end, axis = 80, 930, 11.0, 24.5, 214
    at = lambda hour: left + (hour - start) * (right - left) / (end - start)
    red, blush = KINDS["danger"]
    loaded = [(11 + 13.5 / 60, 11 + 46.5 / 60), (13 + 45.9 / 60, 21 + 23.9 / 60), (21 + 38.15 / 60, 22 + 51.3 / 60)]
    # (hour, row, anchor, text, kind): rows above the axis are negative.
    marks = [
        (11 + 13.5 / 60, -2, "start", "11:13 loaded", "idle"),
        (13 + 45.9 / 60, -2, "start", "13:45:54 loaded by a request", "danger"),
        (21 + 38.15 / 60, -2, "end", "21:38:09 loaded by a request", "danger"),
        (24.05, -1, "end", "00:03 stopped", "idle"),
        (11 + 46.5 / 60, 1, "start", "11:46 unloaded", "idle"),
        (21 + 23.9 / 60, 1, "end", "21:23:54 unloaded", "idle"),
        (22 + 51.3 / 60, 1, "start", "22:51 unloaded", "idle"),
        (21 + 24.0 / 60, 2, "end", "21:24:01 restarted with load-on-startup = false", "idle"),
    ]
    rows = {-2: 112, -1: 140, 1: 268, 2: 296}
    parts = [f'  <line x1="{left}" y1="{axis}" x2="{right}" y2="{axis}" stroke="{MUTED}" stroke-width="1.5"/>']

    for hour in range(11, 25):
        x = at(hour)
        parts += [f'  <line x1="{x:.1f}" y1="{axis - 5}" x2="{x:.1f}" y2="{axis + 5}" stroke="{MUTED}" stroke-width="1"/>',
                  text(x, axis + 22, f"{hour % 24:02d}:00", "note", "middle")]

    for begin, finish in loaded:
        parts.append(f'  <rect x="{at(begin):.1f}" y="{axis - 34}" width="{at(finish) - at(begin):.1f}" height="28" rx="6"'
                     f' fill="{red}" opacity="0.88" filter="url(#lift)"/>')

    for hour, row, anchor, body, kind in marks:
        x, y = at(hour), rows[row]
        edge = axis - 36 if row < 0 else axis + 30
        tip = y + 6 if row < 0 else y - 14
        parts += [f'  <line x1="{x:.1f}" y1="{tip}" x2="{x:.1f}" y2="{edge}" stroke="{KINDS[kind][0]}" stroke-width="1" stroke-dasharray="2 3"/>',
                  text(x + (-5 if anchor == "end" else 5), y, body, "note", anchor, KINDS[kind][0])]

    parts += [
        f'  <rect x="{left}" y="324" width="14" height="14" rx="4" fill="{red}" opacity="0.88"/>',
        text(left + 22, 335.5, "the model in memory", "note"),
        text(left, 362, "The 21:24 restart set load-on-startup = false. Fourteen minutes later the model was back: the setting only", "note"),
        text(left, 379, "governs startup, and with autoload on, router mode loads a preset model for any request that names it.", "note"),
    ]
    save(OUT / "nano-reload-timeline.svg", 1000, 400,
         "A timeline of 2026-10-10 from 11:00 to midnight. Nemotron 3 Nano was loaded 11:13 to 11:46, loaded again "
         "by a request at 13:45:54 until 21:23:54, llama-server was restarted at 21:24:01 with load-on-startup "
         "false, and a request loaded the model again at 21:38:09 until 22:51. llama-server was stopped at 00:03.",
         "Nemotron 3 Nano in memory on 2026-10-10",
         "From the llama-server journal. The two loads in red were started by a request, not by a person.", parts)


serving()
fit()
arguments()
timeline()
print("wrote 4 figures to", OUT)
