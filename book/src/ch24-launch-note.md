<img class="cub" src="art/cub.svg" alt="The Thor Tigress Cub">

# The cub is open

I keep a small server on a desk in my apartment, and it has become the most
interesting thing in the room. It is a Jetson AGX Thor: one board with 128 GB of
memory shared between its processor and its graphics chip, warm to the touch
while it works. It runs two engines and holds four models, two of them in
memory at a time. A conversation can use whichever of them you pick.

```text
$ thor-tigress-serve list

Thor: 59.3 GB free of 122.8 GB · keeps 8 GB free · at most 2 models loaded

  key  model                                          state                 size
  1    Nemotron 3 Nano 30B A3B · Q8_0                 loaded             33.6 GB  default (nemotron)
  2    Nemotron 3.5 Lightning 30B A3B · Q8_0          on disk            35.0 GB
  3    Qwen3.6 27B · Q4_K_M                           on disk            16.8 GB
  4    Qwen3.6 35B A3B NVFP4                          loaded             23.4 GB  TensorRT Edge-LLM · :8081
```

The two loaded models answer today. `thor-tigress-serve load 2` brings a third
one into memory when the memory allows, and the page's picker offers it from
then on. The default, the Nano, answers at about 53 tokens a second and holds a
million tokens of conversation; Qwen3.6 35B A3B runs on TensorRT Edge-LLM
instead of llama.cpp, and chapter "Model comparison" sets the two side by side
on the same questions. The chat page is at
[`voltforge.tech/thor-tigress-cub`](https://voltforge.tech/thor-tigress-cub),
and a Tailscale tunnel carries each message from there to the desk and the
answer back.

Until this week one key opened the door for everybody. Now each person gets a
key of their own. You leave a name and an email on the invite screen, send me a
message on [LinkedIn](https://www.linkedin.com/in/arpan-pathak-272341424/) or
[X](https://x.com/arpanpathak1996), and I approve the request by hand and send
the key back. No sign-up, no password, nobody in the middle taking notes.

<figure>
<img src="figures/cub-architecture.svg" alt="The browser opens voltforge.tech, whose forwarding page sends it to the .ts.net address; from then on every request goes through Tailscale Funnel to thor-tigress-agent on the Thor at :8080, which holds the key, the picker and the APIs and calls SearXNG at :8888. The picker's model id decides the engine: llama.cpp at :8079 for the GGUF models, or TensorRT Edge-LLM at :8081 for Qwen3.6 35B A3B.">
<figcaption><b>Figure 24.1</b> The road a message takes, from the link to whichever model the picker names, on either engine.</figcaption>
</figure>

<figure>
<img src="figures/cub-welcome.png" alt="The chat page in its light theme: the cub in the middle, a greeting, and four suggested prompts">
<figcaption><b>Figure 24.2</b> The first screen, in the light Cub theme.</figcaption>
</figure>

<figure>
<img src="figures/cub-conversation.png" alt="A conversation in the dark theme: a question, a folded thought line, an answer with a highlighted Rust code block, a stats line, and a reply in progress">
<figcaption><b>Figure 24.3</b> A reply, with its reasoning folded away, code highlighted, and the speed written underneath.</figcaption>
</figure>

## What the Thor keeps

The conversation belongs to your browser, and it stays there. The Thor holds
three things:

| Where | What | Until |
|---|---|---|
| your browser | the conversation, the settings, your key | you clear site data or press **+** |
| the Thor's memory | the conversation a window is answering, so the next message on it is not read twice | the model process restarts or unloads |
| the Thor's disk | one encrypted file: the name, email, status and key from the invite screen | the record is deleted |

The chat server writes one file, the keyring, and a keyring record has no field
for a message (`crates/thor-tigress-keyring/src/store.rs`). Once an answer is
sent, the request is gone; the operating system keeps its usual short journal,
as it does for every service on the machine.

<figure>
<img src="figures/what-is-stored.svg" alt="Three columns. Your browser holds the conversations, the settings and your key, cleared with site data. The Thor's memory holds the reply being written and a small prompt cache, gone on restart. The Thor's disk holds the sealed keyring: one record per person with a name, an email, a status and a key, and the service key the servers use between themselves. No message text is written down.">
<figcaption><b>Figure 24.4</b> The three places anything is held.</figcaption>
</figure>

The page sets no cookie of its own, carries no analytics and no advertising, and
asks for no login.

## How a key is checked

<figure>
<img src="figures/keyring-flow.svg" alt="Browsers and agents send a personal key as a header over HTTPS through Tailscale Funnel. thor-tigress-agent compares it with the active keys in the sealed keyring, in constant time, and forwards the request to llama-server with the service key from api-key, which llama-server checks again. No key, or a revoked one, gets 401 before the model is reached.">
<figcaption><b>Figure 24.5</b> The two keys, and where each one stops.</figcaption>
</figure>

1. The invite screen posts a name and an email to `POST /request`, and the
   record waits with no key.
2. `thor-tigress-serve keyring approve EMAIL` gives that person 24 random bytes
   and prints them once as 48 hex characters.
3. The page keeps the key in local storage and sends it as
   `Authorization: Bearer <key>`. Coding agents send the same header, or
   `x-api-key`.
4. `thor-tigress-agent` compares the key against every active key in the
   keyring, in constant time. A key that matches nothing leaves with `401`
   before a model is reached.
5. A request that matches travels to the model you picked under the service key,
   and the model server checks that one again. Personal keys never go that far.

The keyring itself is sealed with Argon2id and XChaCha20-Poly1305. Chapter
"Keys, and the cryptography under them" opens the file and explains both
algorithms, along with the four things they cannot protect.

## How many people it can serve

Each loaded model keeps its own weights and its own pool of windows. The Nano
takes 33.6 GB of weights and about 24 GiB for four million-token windows; the
two together measured 57.8 GB while the chat was serving, against 128 GB on the
board. How that pool is cut is a setting. Today it is cut like this:

| Windows open at once | Tokens each | Pool |
|---|---|---|
| 4 | 1,048,576 | 24 GiB |
| 8 | 524,288 | 24 GiB |
| 16 | 262,144 | 24 GiB |
| 32 | 131,072 | 24 GiB |

The arithmetic is `windows × tokens × 6 KiB`, and memory is the ceiling: ask for
longer conversations and fewer fit at once, ask for more at once and each one is
shorter. One conversation can reach a million tokens, about three novels.
`MODELS_MAX` caps how many models sit in memory at the same time, and both
`load` and `install` refuse a change that would eat the free-memory reserve.
How many people hold a key has no limit at all, because a window belongs to a
reply, not to a person. When every window is busy, the next request waits in the
queue. Chapter "Memory, context and slots" has the measurements behind the
table and the flags that cut the pool differently.

There is no daily allowance and no per-person quota. Someone with a key can keep
the board occupied for as long as they keep asking, and the answer to that is a
revoked key rather than a rate limiter. The board also runs warm while it works,
which in a cold room is a small consolation.

## What it is made of

| Piece | What it does | Listens on |
|---|---|---|
| `llama-server`, router mode | one process per loaded GGUF model: the Nano, Lightning and Qwen 27B | `127.0.0.1:8079` |
| TensorRT Edge-LLM | a second engine, for Qwen3.6 35B A3B in NVFP4 and its successors | `127.0.0.1:8081` |
| `thor-tigress-agent` | the page, the model picker, the OpenAI and Anthropic APIs, the keys, the invite form | `127.0.0.1:8080` |
| SearXNG | web search, when **Web** is on | `127.0.0.1:8888` |
| Tailscale Funnel | HTTPS from the internet to port 8080, and to nothing else | public |

On the Thor, everything lives under `~/.config/thor-chat/`:

| File | Contents |
|---|---|
| `models.ini` | the router's list of models, written by `thor-tigress-serve` |
| `env` | `MODEL`, `USERS`, `CONTEXT`, `MODELS_MAX`, `EDGE_MODEL`, ports |
| `api-key` | the key the agent and the model servers use between themselves |
| `keyring` | who may chat, sealed |
| `keyring-passphrase` | what opens the keyring |

One command, `thor-tigress-serve`, manages all of it:

| Task | Command |
|---|---|
| find, load, unload, download a model | `list`, `list-latest`, `load KEY`, `unload KEY`, `download KEY` |
| install the services and start them at boot | `install` |
| create the keyring | `keyring-init` |
| see who asked, approve, revoke | `keyring requests`, `keyring approve EMAIL`, `keyring revoke EMAIL`, `keyring revoke-all` |
| follow the logs | `logs` |

The crates sit in the repository under Apache-2.0: `thor-tigress-agent` for the
server, `thor-tigress-keyring` for the registry, `thor-spark-safety-eval` for
the prose and code checker, `thor-hammer-trainer` for the training set,
`thor-tigress-reinforcer-frontend` for the review page, and
`thor-lasso-distiller` for conversations drawn out of books. Chapters "Model
serving", "TensorRT Edge-LLM" and "Operations: recovery and hardening" walk
through running the whole thing yourself, on this board or another one.

## What comes next

The launch is close, and the cub is the first piece of it.

Next comes an agentic canvas: a sandbox where a model works on a canvas instead
of a chat box, with tools it may reach for. The uses I am building toward:

| | |
|---|---|
| roleplay | characters who remember the scene, with the scene kept on the canvas |
| a therapy room | a calm listener, with nothing written down on the server |
| immigration | forms and covering letters drafted with you and checked against the rules |
| creative writing | story bibles, drafts and revisions standing side by side |
| music | a timeline the model can add notes to and play back |
| vector graphics | drawing commands it can run and then look at |

Each of those is the same board, a different interface, and a different set of
tools. The book marks what exists in green and what is planned in amber; the
canvas is amber.

## Where this promise stops

- The operator can open the keyring. Whoever holds the Thor's account and the
  passphrase file can read every name, email and key inside it. Encryption
  protects a copy of the file that leaves the machine; on the machine itself,
  the passphrase file is the weak point.
- The invite form is open, and it can be filled with junk. A person approves
  each request by hand, so junk collects in the waiting list and goes no
  further. Every request also costs the board an Argon2id run.
- A key is a password. Anyone holding one chats as the person it was sent to,
  until the key is revoked.
- The windows serve the page and every coding agent together.
- There is one board. It can be busy, offline, or out of memory, and when it is
  down the forwarding page still loads.

## Getting a key

Leave a name and an email on the invite screen, then send a message:

- [DM on LinkedIn](https://www.linkedin.com/in/arpan-pathak-272341424/)
- [DM on X](https://x.com/arpanpathak1996)

The key that comes back is 48 characters long, sent once, and worth keeping
private.

## Elsewhere in this book

- [Security](ch10-security.md) sets out what is exposed and what guards it.
- [Keys, and the cryptography under them](ch25-keyring-crypto.md) opens the
  keyring file byte by byte.
- [What a context window is](ch26-context-window.md) explains why a
  conversation has a limit at all.
- [Memory, context and slots](ch20-memory-and-context.md) measures that limit on
  this board.
- [Model serving](ch21-model-serving.md) lists the models, loads and unloads
  them, and explains the picker.
