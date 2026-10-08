# Worklog

What was done, what broke, and what is still open, newest first. Read this
before changing anything on the Thor. Commit hashes point to the details.

## Machines

| Machine | Role |
|---|---|
| yahboom (Orin NX 16 GB, JetPack 6) | development; `thor-sync` mirrors edits to the Thor within seconds |
| Thor (`ssh thor`, AGX Thor 128 GB) | `thor-chat` (llama-server, Nemotron 3 Nano Q8_0, :8079), `thor-tigress-agent` (page, API, search, :8080), SearXNG (:8888) |

`thor-sync` copies `.git` too but excludes `/book/`, so `git status` on the
Thor lists every book file as deleted. Nothing is lost; the book lives in the
repository and on GitHub Pages. It never deletes on the Thor either, so
folders removed here (`jetson-thor/landing/`, `jetson-thor/site/`) linger
there as untracked.

## 2026-10-07

### The answer rounds run a call again: the text-only rule is reverted

- Report: after the change below, a resume job search stopped giving job links,
  and one reply tripped into repeating a form line. The rule that did it was
  mine: an answer round that wrote text beside a call ended there. It does stop a
  model that keeps calling tools, but it also stops a model that writes the first
  sentence of the summary and then asks for one more page — the reader gets
  "Here are the jobs." and nothing else. Reverted: a call written on an answer
  round is run like any other, as it was before. The other two parts stand and
  neither can cut an answer short: the search hint still comes back out of the
  system line before the answer rounds, and the last-resort note still names the
  sources. `ANSWER_AGAIN` is gone with it.
- Measured, "here is my resume … find NVIDIA job openings in the US that match
  it, summary with links", Nemotron, through the agent on the Thor:
  - thinking off: 4 searches, 12 pages read, 350 s, 3,591-character answer, 5 links.
  - thinking on with the page's 1,024-token budget: 4 searches, 0 pages read,
    64 s, 2,852-character answer, 5 links.
  Both ended with 0 fallback lines and 0 `<tool_call` occurrences. The refusal and
  the repeated line did not reproduce, so the revert removes the only rule that
  could produce them; it is not a confirmed repair of that exact reply.
- Found while measuring, and left alone: the crawl spends the page budget on site
  chrome. In the thinking-off run 6 of the 12 pages read were LinkedIn `/login`,
  `/signup`, `/uas/request-password-reset` and `/company/nvidia`, reached by
  following the job page's own navigation links. Those pages are forms, and form
  text is what a small model repeats. `fetch.rs` and `html.rs` are untouched: if
  the page budget should go to postings instead, that is a rule to design in
  those modules, not something to bolt on here.
- The entries below still describe the text-only rule; this one supersedes them.

### Think: a budget for the overthinking, and no more empty bubble

- Report from a live chat: with Think on, a hard question made the model
  overthink and end with no answer at all. Cause, measured: nothing bounds
  thinking. llama.cpp's `reasoning_budget_tokens` defaults to `-1`, its
  `n_predict` defaults to `-1`, and the page sent no `max_tokens`, so a model
  that will not leave the think block is bounded only by the 4M-token slot.
  On the Thor, Nemotron, thinking on, "lock-free MPSC queue in safe Rust",
  `max_tokens` 2048: `finish_reason: length` after exactly 2,048 completion
  tokens, the answer cut off mid-sentence, 4,780 characters of reasoning before
  the first word of it. The reply the report showed was also exactly 2,048
  tokens; no cap exists in the page or the agent, so that number is the model
  stopping on its own after 2,048 tokens of thinking. Either way, unbounded.
- Fix, in the chat page. Think now sends `reasoning_budget_tokens` and
  `reasoning_budget_message`, the line llama.cpp puts in front of
  the end-of-thinking tag when the budget runs out. The budget is a settings
  field, 1,024 tokens by default, empty for no limit. Measured through the agent
  on the prompt above: budget 256 stops the reasoning at 1,172 characters, the
  injected line arrives, and 15,661 characters of answer follow.
- The page no longer shows an empty bubble when a reply is all thinking: the
  reasoning opens by itself and a line says the model thought but wrote nothing,
  naming the stop reason when it was the token limit. The stats line under a
  reply now carries `stopped at the token limit` instead of hiding
  `finish_reason`.
- Not fixed, and not fixable from the page: the same session had the model invent
  compiler errors — it said `.iter().filter(|x| x % 2 == 0)` does not compile and
  printed a rustc error that rustc does not have. That is the model lying, which
  is what Stages 2 and 3 are for. `spark` reads Rust source, and a false sentence
  in prose is not Rust source. The immediate things that exist: the model picker
  (Qwen3.6-35B-A3B-NVFP4 is served on `127.0.0.1:8081`) and the system prompt
  field.

### SearXNG: the engines behind the search were off, not blocked

- The loop fix below ended in a written answer that said it found nothing, and it
  was right: every search returned 0 results. SearXNG's own answer named
  `brave: Suspended: too many requests`, `duckduckgo: CAPTCHA`,
  `google cse: Suspended: too many requests`.
- Cause: the shipped `searx/settings.yml` has nearly every general engine
  `disabled` or `inactive`, and the Thor's `~/.config/searxng/settings.yml` used
  `use_default_settings: true` without enabling any. Only `brave`, `duckduckgo`
  and `google cse` were on, and all three are rate-limited from this address, so
  a search could only ever come back empty. The engines were never the problem:
  `mojeek`, `bing`, `qwant` and `wiby` answered `200` to a plain `curl` from the
  Thor.
- Fix, in `~/.config/searxng/settings.yml` (backup:
  `settings.yml.bak-2026-10-07-engines`; `systemctl --user restart searxng`):
  enable the no-key engines that answer from this address. Ten do: `bing`,
  `yahoo`, `encyclosearch`, `fynd`, `mwmbl`, `privacywall`, `reloado`,
  `resulthunter`, `seznam`, `wiby`. Measured with `format=json`: "nvidia jobs"
  144 results from bing, encyclosearch, fynd, mwmbl, privacywall, reloado and
  wiby; "senior rust engineer remote" 71; "nvidia site:jobs.nvidia.com" 29, the
  first hit `https://jobs.nvidia.com/careers`.
- `mojeek`, `startpage` and `dogpile` cannot be switched on: they are `inactive`
  upstream (proof-of-work CAPTCHA). `marginalia` needs an API key. `qwant`,
  `gabanza`, `tusksearch`, `fireball`, `fastbot`, `searchmysite`, `ayo` and
  `crowdview` were tried and left off — CAPTCHA, a certificate error, a 401, or
  empty. `brave`, `duckduckgo` and `google cse` are still suspended; they are the
  defaults and stay on so they return if the block lifts.
- The same live query through the agent, after: 4 searches, 12 pages read, and a
  3,928-character answer — a table of 7 NVIDIA roles, each with a link and a `[n]`
  citation — in 334 s, with 0 fallback lines and 0 `<tool_call` occurrences in the
  stream. This is the run that used to end on the fallback.
- Open: several of those roles come from aggregators (hitmarker.net,
  claveprep.com, beyond-tabs.com) rather than the company's own posting, so the
  answer is only as good as the snippets. Getting `brave`, `duckduckgo` or Google
  back would need an API key or an outbound proxy.

### The answer rounds are the model's turn to write, and nothing runs in them

- Report from a live job search: the reply was the fallback "I ran out of tool
  rounds without a written answer". Cause, in `search_loop`: the answer rounds
  were sent with no `tools`, but a call the model wrote there was still run and
  appended. A model that kept wanting to search (Nemotron does) spent all three
  answer rounds calling tools and never wrote a word. The system line made it
  worse: `SEARCH_HINT` tells the model to plan sub-questions and call
  `web_search`, and it stayed in the system turn for the answer rounds, where it
  outranks the last user ask.
- Fix, in `chat.rs`. New `retract_system` takes `SEARCH_HINT` back out of the
  conversation's one system message before the answer rounds, leaving
  `ANSWER_NUDGE` and the numbered source list. The answer rounds are text-only
  now: a call written there is dropped, nothing runs, and the model is asked
  again with the new `ANSWER_AGAIN` line. Text written beside a call is the
  answer and the call is not run, so a reply is never held behind a call.
- The last-resort note no longer says "ask again". When no round wrote anything
  it names the first ten sources the ledger found, so the answer still ends with
  something to use.
- Tests: 168 in the agent, up from 163. New rows: a call on an answer round is
  dropped, not run, and the model is asked again; text beside that call is the
  answer; the note names the sources when no round writes one; the hint is
  retracted from the front, the middle and the whole line, and the retraction is
  a no-op when the hint was never added; the answer-round request no longer
  contains the hint. `cargo test --workspace` passes, `clippy --workspace
  --all-targets -D warnings` is clean, `spark rs crates` finds 0 problems in 83
  files.
- Docs: the book chapter "Tool calling" (steps, the text-call section, the
  limits table, the tests table), the figure alt text, the README search section
  and the WORKLOG were updated. The answer phase is three rounds, not two, and
  no call runs in it.
- Deployed to the Thor, 2026-10-07 23:39 PDT: backed up the old binary
  (`~/.cargo/bin/thor-tigress-agent.bak-2026-10-07-answerphase`), ran
  `cargo install --locked --force --path crates/thor-tigress-agent` (3.3 s),
  `systemctl --user restart thor-tigress-agent`, and `/health` answered
  `{"status":"ok"}`. A live Web-on request (Nemotron 3 Nano, the same job-hunt
  shape that failed before: NVIDIA openings for a senior systems and Rust
  engineer, a link each) then ran 16 searches and wrote a 110-character answer.
  The raw stream held 0 occurrences of `<tool_call` and 0 fallback lines, where
  the old binary ended on the fallback.
- Measured on the same run: all 16 searches returned 0 results. SearXNG answered
  `/search?q=nvidia+jobs&format=json` with `0` results and
  `brave: Suspended: too many requests`, `duckduckgo: CAPTCHA`,
  `google cse: Suspended: too many requests`. So the loop fix is measured as
  "a written answer arrives", not yet as "a good answer from sources"; the model
  was right to say it found nothing. The suspended engines are the same incident
  as the 2026-10-07 entry below and are still open.
- Not measured yet: how often a fresh question reaches the answer rounds with a
  useful answer, and whether Nemotron ever writes text beside a call on an answer
  round outside the unit tests.

### Web search made useful for both models, and reading the pages

- `web_search` gained an optional `time_range` (`day`, `week`, `month`, `year`,
  plus `today`, `7d`, `30d`, `12m`), passed to SearXNG as its own parameter, and
  now keeps SearXNG's `publishedDate` in the text the model reads. The tool
  description and the Web-on system line ask for a range when the answer depends
  on what is recent, so the chat can find jobs and new postings.
- The first round of a **Web**-on answer is sent with
  `tool_choice: "required"`. This is the guaranteed fix chapter "Model
  comparison" identified (Nemotron searched 15 of 18, Qwen 3 of 18): both models
  now have to look once before they answer, and later rounds return to `auto`.
- New tool `fetch_page_content_recursive`. One cited address, then the links on
  that page's own site, two hops deep and six pages at most; each page cut to
  12,000 characters, 24,000 in all. It fixes the measured cuda-oxide case where
  the model saw snippets only.
- New modules, each one job: `address.rs` (https address type, the public-address
  table, one lookup), `html.rs` (HTML to text, title, same-site links),
  `http.rs` (the `Web` trait, ureq with a pinned resolver, limits, fixed
  headers), `fetch.rs` (rule 1 for links, the crawl, redirects, the untrusted
  label). `chat.rs` gained the second tool, the allowed-host set built from the
  user's message and from each search, and a `read:` event. `search.rs`,
  `config.rs`, `testing.rs` and the chat page were updated with it.
- New dependencies, asked for first: `ureq` (rustls) and `html2text`; tests add
  `rcgen` and `rustls` to serve a real TLS page with a self-signed certificate,
  so the client is tested over a handshake rather than a mock.
- The safety rules of chapter "Tool calling" are implemented and tested: private,
  loopback, link-local, shared, multicast and reserved ranges (including
  `::ffff:127.0.0.1`, `64:ff9b::/96`, `100.64.0.0/10`), one lookup and one
  connection, three redirects by hand, relative `Location`s resolved against the
  page, 2 MB and 10 s per page, text types only, no cookies, `Authorization` or
  referrer. Rule 1 for links is tightened: the start address must come from a
  search result or the user's message, and links must stay on that page's site.
- Verified on yahboom, 2026-10-07: `cargo test --workspace` passes; `clippy
  --workspace --all-targets -D warnings` clean; `spark rs crates` finds 0
  problems in 81 files; the agent runs 126 tests. `mdbook build book` is clean,
  all 49 figure references resolve, and every SVG parses.
- Deployed to the Thor, 2026-10-07 10:08 PDT: backed up the old binary
  (`~/.cargo/bin/thor-tigress-agent.bak-2026-10-07-websearch`), ran
  `cargo install --locked --force --path crates/thor-tigress-agent` on the Thor
  (15 s), restarted `thor-tigress-agent`, and `/health` answered
  `{"status":"ok"}`. Two real Web-on requests then ran. The first searched three
  times with `time_range: "week"` and got six results each, the first
  `Announcing Rust 1.99.0` at `blog.rust-lang.org`. The second read
  `https://blog.rust-lang.org/releases/latest/`, followed that page's own link to
  `/2026/10/01/Rust-1.99.0/`, and answered "Rust 1.99.0" naming the page. The
  refusal cases are covered by the unit tests, not by a live run.
- Rollback if needed: the `.bak` binary, or `cargo install` the previous commit.
- Still open: rule 8, the fetch line in the journal, is not written yet; no
  cache; no measurement yet of how often each model answers a fresh question
  correctly with the forced first round.

### Leaked tool calls: `tooltext.rs`, and a real answer round

- Report from a live answer: the reply was a raw
  `<tool_call><function=fetch_page_content_recursive>…</function></tool_call>`
  block, and no tool ran. Cause: `search_loop` offered tools only while
  `round_number < MAX_ROUNDS`, so the fourth request had no `tools`. A model that
  still wanted one wrote the call into `content`, and the loop forwarded it
  verbatim.
- Fix, in three parts. New module `tooltext.rs`: a streaming filter (`Sieve`)
  that holds back anything that could be a tool-call tag, parses a complete
  block in Nemotron's XML form or the JSON form, and returns it to the loop as if
  it had arrived in `tool_calls`. The tags never reach the page, and a tag split
  across two chunks comes out whole.
- `chat.rs`: tools are now offered on all four tool rounds; a fifth round runs
  without tools and with one line telling the model to answer, so the reply is
  always plain text. A repeat of a call already run is not run again; the model
  is told and asked to use what it has. The answer round's own text is filtered
  too, and a short fallback is sent when nothing else arrived.
- Tests: 137 in the agent now, including a call written as text (run, tags not
  shown), a call split across chunks, a repeated call, and a call leaked on the
  answer round. Chapter "Tool calling" gained "Calls written as text" and the
  test rows; the README section was updated.
- Second report, same day: a search for several job titles ended in the fallback
  "I ran out of tool rounds without a written answer". Cause: four tool rounds
  is too few for a hunt, and the single answer round was itself a tool call,
  which the filter dropped, so there was nothing left to show.
- Follow-up fix: tool rounds raised to six (`MAX_ROUNDS`), and the end is now a
  phase of up to two rounds without tools (`ANSWER_ROUNDS`) in place of one. A
  call the model writes in that phase is run, its result is added, and it is
  asked again; the fallback is sent only when two rounds yield no text at all.
  Agent tests 138; the round count in the book, the README and the tool-loop
  figure was updated to six.
- Deployed and checked live, 2026-10-08: "search recent remote senior Rust
  engineer jobs, and senior distributed systems engineer jobs; list openings
  from the last month with company and link". The agent searched twice (once per
  title) and answered with a 2,108-character list; the stream held 0
  occurrences of `<tool_call` and 0 fallback lines.
- Deployed to the Thor again, 2026-10-07: rebuilt and reinstalled, service
  restarted, `/health` ok. A live Web-on request ("search the latest Rust
  release, read the announcement, name the page") then searched twice, read the
  GitHub releases page and `blog.rust-lang.org/releases/latest/`, and answered
  "Rust 1.99.0, announced on October 1 2026 … stabilization of C-variadic
  functions", naming the page. The raw stream held 0 occurrences of
  `<tool_call`.

### Deep research: sub-questions, kinds, and a ledger

- Asked not to "just make it 6-8". The round count alone is not research, so
  this is the machinery around it.
- New module `research.rs`. `Kind` widens one query into the few that find what
  a hunt needs: `jobs` for postings, `people` for the recruiter and the hiring
  manager behind one, whose own posts often name a role first. `Ledger` gives
  every source one number, from 1, keeps that number when the same address turns
  up again, and holds the budgets: 16 searches and 12 pages per answer.
- `web_search` gained `queries` (up to three more sub-questions, searched in the
  same round) and `kind`. Each query's hits are numbered from the ledger, so the
  answer can cite `[7]` for a source found in the second round; every tool result
  ends with `Searches used 3 of 16; pages read 1 of 12`; a call over the budget
  is answered with the budget line instead of running, and the tool rounds stop
  once the search budget is spent.
- `fetch_page_content_recursive` now takes the pages left in the budget, so a
  reading-heavy answer cannot run away. `read_recursive` takes the limit as an
  argument, defaulted to six by `MAX_PAGES` for a single call.
- The system line under the switch now asks for the plan before the searches:
  sub-questions, one search each, the people as well as the postings, read the
  best pages, then answer with headings and a citation per claim. Before the
  answer rounds the whole numbered source list is added to the system message.
- Rounds raised to eight, and three answer rounds; the ask is repeated as the
  last user turn, which is the turn a model that keeps calling tools reads.
- First live run against the real web exposed the coarse version: widening added
  "hiring" to a query that already said hiring, and 12 searches went on empty
  angles before any page was read, so the answer never came. Widening now keeps
  the model's own query first and skips an angle it already covers, the search
  budget is 16 with an early stop when it is gone, and the answer ask is a user
  turn.
- `search` now returns `Hits`: the results and SearXNG's `unresponsive_engines`.
  When the engines are suspended or showing a CAPTCHA the tool text says so, so
  the model can explain a thin answer instead of guessing. Found the hard way:
  the first live research run returned 0 results for all 16 searches because my
  own testing had got DuckDuckGo a CAPTCHA and Brave and Google suspended.
- Second live run with the engines still down: 16 searches, 0 results, and a
  structured answer that said so, listing what it had searched and that it had
  no verifiable facts - no fallback line, no leaked tag. The engines will answer
  again once the suspensions lift; until then a Web-on answer is honest and thin
  rather than wrong.
- Tests: 163 in the agent, 442 in the workspace. Chapter "Tool calling" gained
  "Deep research: sub-questions, kinds, and a budget"; the README and the
  tool-loop figure follow.

### Design only: sharing the Thor with a friend

- A friend wants GPU time and somewhere to host their apps, with their own domain.
  Nothing is built; this is the design and the checklist for the afternoon it
  gets built. Chapter "Sharing is caring", three figures.
- Decided: their own Unix user, with sudo (the owner's call) and the `video` and `render`
  groups; reaching the Thor by sharing the node in Tailscale, so no port opens
  at home; Tailscale Funnel and serve for their apps, not Cloudflare; their domain
  in front of a Funnel path by a forwarding page, because Funnel serves only the
  node's own `.ts.net` name and its certificate.
- The chapter records what sudo buys a second person on this box (the chat's api-key file,
  the projects, the SSH key if it has no passphrase, the ability to stop the
  chat services) and what stays the owner's (the keyring, sealed with the owner's passphrase).
  The SSH key is named as the one thing to settle before this starts.
- Facts checked on the Thor: node `arpanpathak.taildb9a39.ts.net` at
  `100.84.254.65`, Tailscale 1.102.4, no operator set, Funnel 443 to
  `127.0.0.1:8080`, four `--user` services, 742 GB free, one GPU.
- The node name came up: `arpanpathak.taildb9a39.ts.net` would be in every
  address a recruiter clicks. Chapter "Operations" already documents a rename,
  and the machine name is free text (`tailscale set --hostname=`, or the admin
  console), while the tailnet name can only be swapped for one of Tailscale's
  random pairs. Both belong to the whole node, so a rename moves the chat's own
  address, the `voltforge.tech` forwarding page, the Claude Code and
  openBatarangs aliases, `about.html` and the book with it.
- Because of that the design now recommends a node of their own in front of their
  apps instead: their Tailscale account, their Funnel, their certificates, a short
  proxy to the Thor over the shared node. Their links carry no name of the owner's, and
  none of the owner's addresses move. A container or small VM on the Thor does the same job
  without extra hardware, at the cost of a second `tailscaled` on the host.
  Figure 2 draws that route; figure 3 keeps the single-node one.
- Still open: whether the friend runs that node or it is a container here, and
  the port range.

### Per-person keys: thor-tigress-keyring, and the registration form

- New crate `thor-tigress-keyring` (library and binary). One encrypted file,
  `~/.config/thor-chat/keyring` (mode 600): a magic line, a random salt, a
  random nonce, and the JSON records sealed with XChaCha20-Poly1305 under a key
  Argon2id stretches from the passphrase. A record is a name, an email, a
  status (`requested`/`active`/`revoked`), the key and the time. Commands:
  `init`, `request NAME EMAIL`, `requests`, `approve EMAIL`, `keys`,
  `show EMAIL`, `revoke EMAIL|KEY`, `export`. The passphrase comes from
  `--passphrase-file`, `THOR_KEYRING_PASSPHRASE` or a line on standard input;
  every write uses a fresh nonce and a temporary file renamed into place. New
  dependencies, asked for first: argon2, chacha20poly1305, getrandom, zeroize.
- `thor-tigress-agent`: `--keyring FILE` and `--keyring-passphrase-file FILE`.
  The keyring's active keys let visitors in; they are decrypted in memory and
  reloaded when the file changes, so approving someone needs no restart.
  `--key-file` stays the one key the agent sends to llama-server, so personal
  keys never reach it. `POST /request` (no key) records the registration form;
  an email already waiting is not added twice.
- `jetson-thor/web/index.html`: the invite screen now has a name and email form
  that posts to `/request`, and links to DM on LinkedIn and X. The single-key
  field stays for people who already have one.
- `thor-tigress-serve`: `keyring-init` makes the keyring and a random passphrase
  file; `keyring CMD ...` runs the Rust tool on the same files; `agent` passes
  both to the agent when they exist.
- Measured on yahboom, 2026-10-07: 360 tests pass in the workspace; clippy
  clean; spark finds 0 problems in 77 files. End to end on real files: init,
  request, approve (a 48-hex key), the agent's `/health` ok, `/request`
  recorded a second person and answered 400 for a bad email, `/v1/models`
  answered 401 without a key and 502 with one (no llama-server on this
  machine), and `revoke` emptied `export`. Coverage on the CI metric: keyring
  99.5%, agent 98.6% lines; `llvm-cov show` reports no uncovered lines, the
  same phantom-line gap the other crates show with this toolchain.
- `revoke-all` was added on 2026-10-07, so the launch note's "one command
  invalidates every key" is true rather than a plan: it marks every active key
  revoked, keeps the records, and takes effect on the next request. With it,
  the keyring runs 28 tests.
- Still open: no per-person rate limits; the keyring is only as safe as the
  passphrase file beside it; `POST /request` is unmetered (gap 13 in the
  operations chapter).

### The book: launch note, keys and the context window

- Three new chapters, written to be checked against the code:
  - "Launch note: what we store, and what we do not": conversations in the
    browser, the Thor's disk holding only the keyring, why there are no rate
    limits, one-command revocation, and the limits of that promise.
  - "Keys, and the cryptography under them": Argon2id (why a passphrase is
    stretched and not hashed, salt, the parameters), XChaCha20-Poly1305 (AEAD,
    a fresh nonce, associated data, one refusal for two mistakes), the keys
    themselves (24 random bytes, constant-time compare, wipe on drop), atomic
    writes, what encryption here does not protect, and the RustCrypto crates by
    name.
  - "What the context window is": the window as a fixed budget per reply, why
    every message re-sends the whole history, what KV means with the
    key/value/query picture, what fills it, and what to do when it is full.
- Eight new SVG figures in the house style: the storage map, the keyring file's
  bytes, the KDF, the seal and open, the key's life, the request flow, the
  window, and the KV cache. The single-key flow figure was replaced by
  `keyring-flow.svg`, and `key-flow.svg` was removed.
- Corrected the chapters the change made wrong: the web chat's invite screen and
  access-key sections, the security chapter (Figure 12.1 and the three keys),
  the operations runbook (the leaked-key steps now revoke keys instead of
  rotating everyone) and its gap table (gap 1 closed, gap 13 added), the agent
  chapter's "Get a key", and the About page's invite section.
- Verified: `mdbook build book` clean; 35 SVGs parse as XML; every figure
  reference resolves.
- Rewritten after review, same day. The launch note now opens with the cub art
  and carries the path figure, the welcome and conversation screenshots, the
  storage figure, and the tooling: the services and ports, the four files under
  `~/.config/thor-chat/`, the commands, and the crate list. The context-window
  chapter is shorter, keeps the arithmetic in chapter "Memory, context and
  slots" where the measurements are, and drops the desk metaphor from the
  figure title. Phrasing that read as filler ("checked against the code, not
  taken on trust", "the feature, not a cost") is gone.
- Rewritten a second time the same day, against Joseph Williams' *Style:
  Lessons in Clarity and Grace*: characters in subject position, actions in
  verbs, old information before new, 15–20 word sentences with varied length,
  no hedging. The launch note now opens with the board on the desk, and the
  four-reply framing is gone. The live machine runs `--ctx-size 4194304` across
  `--parallel 4`, so the number of replies at once follows from the memory
  pool; the note gives the four shapes of that pool and names memory as the
  ceiling. A new section sets out what comes next: the launch, and the agentic
  canvas with roleplay, a therapy room, immigration paperwork, creative
  writing, music and vector graphics.
- Rewritten a third time, after the note was still overfitted to one model and
  led with the engine rather than the product. The opening now shows the thing
  people use: four screenshots (Paper, Night, a phone, and the invite door), a
  list of what the page does, and the themes by name. The board and its four
  models moved down into "What it is made of", where the `thor-tigress-serve
  list` output from the Thor and `MODELS_MAX` now live. `cub-architecture.svg`,
  the system figure in chapters "Web chat: Thor Tigress Cub" and the launch note
  (Figures 9.6 and 24.7), draws both engines: the agent routes by model id to
  llama.cpp at :8079 or TensorRT Edge-LLM at :8081, with two arrows instead of a
  chain that implied one fed the other. `keyring-flow.svg` lost the stray red
  elbow that crossed the whole figure; the 401 now sits in the agent box, with a
  short red line back to the client. Chapter "Model serving" keeps its figure as
  the router's own view, with a line saying where the second engine is covered.
  The release is named `thor-tigress-cub-junior` (Haloom!) in the launch note,
  the introduction and the About page footer.

### Deployed to the Thor

- Order: backed up the old agent (`~/.cargo/bin/thor-tigress-agent.bak-2026-10-07`),
  installed `thor-tigress-keyring` then `thor-tigress-agent`
  (`cargo install --locked --force`), ran `thor-tigress-serve keyring-init`
  (`~/.config/thor-chat/keyring` and `keyring-passphrase`, both mode 600), then
  restarted `thor-tigress-agent`. `api-key` was not touched, on purpose.
- Verified on the Thor, 2026-10-07 02:55 PDT: the agent logs "access key
  required, keyring on"; `/health` 200; the **old shared key still answers
  `/v1/models` with 200**, so saved browser sessions and agents keep working;
  no key and a bogus key both get 401; the invite page carries the form and the
  About page the DM links; `POST /request` with a bad email gets 400; the
  waiting list is empty. Over the internet: the `.ts.net` page is 200 and
  `/v1/models` is 401, and `https://voltforge.tech/thor-tigress-cub` forwards
  (the trailing-slash form 404s, as GitHub Pages does).
- Rollback if needed: restore the `.bak` binary, remove `keyring` and
  `keyring-passphrase` (both new), restart.
- Next: a first real request from a phone, then `keyring approve EMAIL`. The old
  shared key stays valid until `api-key` is rotated, which is the step that
  retires it.

## 2026-10-06

### Qwen3.6-35B-A3B on TensorRT Edge-LLM, live in the chat

- Edge-LLM v0.11.0 installed from its aarch64 wheel into
  `~/.local/share/edge-llm/venv` (venv made with `--without-pip` plus
  get-pip.py, since `python3.12-venv` and Docker need sudo). Torch 2.13.0+cu130
  sees the Thor (sm_110).
- `nvidia/Qwen3.6-35B-A3B-NVFP4` (23.4 GB) in `~/models/edge-llm/`. The first
  start built the language engine in 270 s and the vision engine in 113 s, and
  was serving after 442 s. Later starts take 30 s from
  `~/models/edge-llm/cache`. It uses about 30 GB at a 32K context, batch 1.
- 20 Rust tasks (`jetson-thor/model-serving/compare.py`), thinking off:
  Qwen 78.0 tok/s, 18/20 compile, 14/20 tests pass, spark all five rules 30%;
  Nano 53.3 tok/s, 15/20, 14/20, 5%.
- Agent: `--engine MODEL=HOST:PORT` routes requests by `model`, and
  `/v1/models` merges the lists. Coverage stays at 100%; 73 tests.
- `thor-tigress-serve`: `EDGE_MODEL`, `EDGE_PORT`, `EDGE_CONTEXT`, the `edge`
  command, the `thor-edge-llm` service (MemoryMax 48G), and an Edge-LLM row in
  `list` that `load`/`unload` start and stop.
- On the Thor: `EDGE_MODEL` is set in `~/.config/thor-chat/env`,
  `thor-edge-llm` is enabled, and the agent was restarted once with no Nano
  reply in progress. `thor-chat` was not restarted. 35 GB free with both.

### thor-tigress-serve: subcommands instead of a prompt

- The prompt ("2 load") is replaced by `list`, `list-latest`,
  `load KEY|NAME`, `unload KEY|NAME` and `download KEY|REPO`. `list-latest`
  saves its list to `~/.cache/thor-tigress-serve/latest.json`, so `download`
  keys refer to the last `list-latest`. Every subcommand was tested on the
  Thor, including a download, load and unload of LFM2.5-VL-3B (deleted
  again).
- The book's "What went wrong once" section was removed at the user's
  request.
- **Engines (researched, nothing installed):** TensorRT-LLM isn't supported
  on Jetson. TensorRT Edge-LLM supports Jetson Thor (JetPack 7.x, CUDA 13),
  and its supported-models page lists Nemotron 3 Nano 30B-A3B NVFP4 and
  Nemotron 3.5 Lightning 30B-A3B NVFP4. v0.11.0 (2026-09-29) has aarch64
  wheels and an experimental OpenAI-compatible server. Installing it and
  building engines is a long GPU job on the shared Thor, so it waits for the
  user's go-ahead.

### Model serving: thor-tigress-serve replaces serve.sh

- `serve.sh` is replaced by `jetson-thor/model-serving/thor-tigress-serve`,
  one Python file using only the standard library, symlinked into
  `~/.local/bin` on the Thor. Run alone, it lists the models with a key
  each; you type "2 load" or "1 unload". `list-latest` adds the newest
  unsloth and ggml-org chat GGUFs that this llama.cpp can run and that fit;
  you type "5 download". The `models`, `memory`, `load NAME`, `unload NAME`
  and `reload` commands are gone, as are the `LIGHTNING*` settings.
- Every GGUF under `~/models/gguf` is listed as "on disk". Loading one adds it
  to `models.local.ini` (1 slot, 64K context); unloading takes it off the
  router's list again, so the web picker only shows what is loaded.
- `load` refuses when `MODELS_MAX` models are already loaded, instead of
  letting llama-server evict the least recently used one (possibly the
  Nano). It also refuses when the file plus 2 GB wouldn't leave
  `MIN_FREE_GB` free.
- Tested on the Thor end to end with LFM2.5-VL-3B (2.9 GB): downloaded,
  loaded in 2 s, answered at 70 tok/s through the agent, unloaded (4.6 GB
  freed), deleted. The guards were tested with `MIN_FREE_GB=200`,
  `MODELS_MAX=1` and "n" to unloading the default.
- The Thor's units now run `thor-tigress-serve run` and `agent`, repointed
  without a restart. The commands they will run were compared with the
  running processes: identical, and `models.ini` was byte for byte the same.
  `~/.config/thor-chat/env` is empty again.

### Later the same evening

- **Thinking panel, second fix.** The first fix (87793db) still moved the
  panel to a new message node on every frame. Chrome and Safari drop a
  click whose press landed on a node that was detached, even briefly. Now
  the panel never leaves the page; only the parts around it are replaced.
  Tested with real pointer input over WebDriver BiDi in Firefox on the Thor
  (press, 150 ms hold, release): before any fix it didn't open; with either
  fix it opened. No Chrome or Safari on either machine, so those weren't
  tested directly. The page is sent with `Cache-Control: no-store`.
- **Lightning switched off** at the user's request (disappointing answers):
  unloaded, then `LIGHTNING=none` added to `~/.config/thor-chat/env` on the
  Thor and `./serve.sh reload`, so it is off the list too. The Nano stayed
  loaded. Memory available: 60.2 GB. The file stays in `~/models/gguf/`.
- **Chat page code:** `updateLast` now patches the streaming message by
  named parts (`data-part`); only the thinking panel is patched in place,
  the rest is replaced. The picker code lost its fake `"model"` entry
  (`showServer`, `showModels`, `chosenModel`). Tested with real pointer
  input, with one model and with two.
- **serve.sh moved** to `jetson-thor/model-serving/serve.sh`. The Thor's two
  unit files were repointed with `sed` + `daemon-reload`, without a restart.
  The old copy on the Thor was deleted by hand (thor-sync never deletes).
- **New commands:** `models`, `memory`, `load` (guarded: one-token warm-up,
  undone below `MIN_FREE_GB`), `unload`, `reload`, plus
  `~/.config/thor-chat/models.local.ini` for models being tried. All were
  tested on the Thor.
- **Book:** chapter "Model serving" (`ch21-model-serving.md`), placed right
  after "Access and syncing"; figures renumbered in the chapters after it.
  The `/slots` commands in ch09 and ch20 now pass `?model=nemotron`, which
  router mode requires.
- **Open question from the user:** why llama.cpp and not TensorRT-LLM or
  TensorRT Edge-LLM. The Thor has TensorRT 10.16.2 (`pip`, `libnvinfer`),
  but no TensorRT-LLM and no container. Support for these models on sm_110
  hasn't been checked.

### Nemotron 3.5 Lightning as a second model

- The model exists: NVIDIA, released 2026-08-11, 30B total / 3B active,
  Mamba-2 + MoE + attention, 1M context, OpenMDW-1.1 licence.
- On the Thor:
  `~/models/gguf/Nemotron-3.5-Lightning-30B-A3B/NVIDIA-Nemotron-3.5-Lightning-30B-A3B-Q8_0.gguf`
  (unsloth GGUF, 35,004,643,392 bytes, size matches Hugging Face). The Thor's
  llama.cpp (8216c84, 2026-10-05) runs it.
- `serve.sh run` now starts llama-server in router mode
  (`--models-preset ~/.config/thor-chat/models.ini --models-max 2`). The Nano
  is unchanged (4 × 1M, aliases `nemotron`, `nemotron-think`, its GGUF path).
  Lightning gets 1 × 256K and the alias `lightning`. The user chose this
  split.
- Chat page: the model chip is a picker filled from `/v1/models`. The choice
  is remembered and sent as `model`. Tested in headless Firefox on the Thor.
- Measured with both loaded: Nano 53.5 tok/s, Lightning 52.5 tok/s (same Rust
  prompt, thinking off, 600 tokens). Both loaded 32 s after a restart; 26 GB
  of memory still available.
- **Behaviour change:** a request without `model`, or with an unknown name,
  now gets 400. Before, the name was ignored. Lasso's default `--model teacher`
  would be refused if pointed at :8079 or :8080.
- **Incident, 20:49–20:52 PDT:** I first loaded Lightning on a test port at
  4 × 1M next to the live Nano. Memory ran out on the first generation and
  the OOM killer took `thor-chat` down. systemd restarted it on the
  router-mode `serve.sh` that thor-sync had already copied (both models at
  4 × 1M), which crash-looped. Fixed by restoring the committed `serve.sh`
  and restarting. Later tests ran in a 48 GB systemd scope with a watchdog,
  and the deploy had an automatic rollback.

### Chat page: thinking panel (87793db)

Clicking "thinking…" while the answer streamed did nothing. The page
rebuilt the last message on every animation frame, so the mouse went down
on one element and came up on its replacement, and the browser dropped the
click. The fix keeps the same panel and updates only its text. Reproduced
and verified in headless Firefox on the Thor with a real press and release
(a scripted `click()` did not show the bug). Deployed by copying the page;
the agent reads it from disk on every request.

### Coverage (2103674)

100.00% line coverage in every crate, 317 tests, CI green. What it took:
`cargo llvm-cov` counts each function by its best single compiled copy, so
generic code and closures that never run show up as misses. They were
replaced with trait objects (`&mut dyn Write`), channels instead of
`join().map_err(...)`, `serve() -> Outcome<Infallible>`, and tests for the
real error paths.

### Live chat prompt and temperature, reverted (3aaf81a, 70aecbd, 7c4aacf, 0e6fe2e)

I gave chat requests a default system prompt and temperature 0.3. Its "at
most three short sentences" line cut every answer short. Reverted at the
user's request and redeployed. Rule since then: no change to how the live
chat answers without the user approving that exact change.

The "Nemotron regressed" report had no server-side cause. It was sampling
at temperature 1.0 with long histories. The rules plus temperature 0.3
improved style but not correctness. The options offered (compile-and-retry,
a stronger coding model, the fine-tune) are still undecided.

### Teacher set (1ad650b, d67fcd8, 361817c)

- `train/teacher/01-10*.md`: 60 hand-written conversations. Keep them; the
  user asked that they never be deleted.
- `train/teacher/grounded/*.md`: conversations grounded in real sections of
  books, docs and open-source code, one file per source, with source,
  section and licence on every entry.
- `teacher pick N` queues real sections; `teacher check` builds, tests and
  rule-checks every fence.
- Totals at the last check: 106 entries pass, 46 of them grounded.
- Not used: the gpu-accelerated-kubernetes book (user's call),
  cpp-core-guidelines (personal-use licence), and gobyexample (licence only
  stated in its README; awaiting the user's call).
- Open courseware is not fetched yet. Check each licence first; NC licences
  are refused.

### Review page (2676c49, b06944b)

`reinforcer` serves train, teacher and conversations on one port
(127.0.0.1:8787), with a dark page, Good (`a`) / Slop (`f`) and categories
on a selection (keys 1–8). Hammer's old review binary is gone.

### Book

ch13 §7.7 (the running-median case study, gaps G1–G6) and §7.8 (the teacher
set), with three figures.

## 2026-10-04 to 2026-10-05

Platform, Stage 0 checker (spark), web chat on the Thor with web search,
the book on GitHub Pages, voltforge.tech forwarding, OpenCode and Claude Code
pointed at the Thor, lasso. See `git log` from 72423d3 to 627358f.

## Rules learned the hard way

- Do only what was asked; no adjacent edits, commits or pushes.
- Large edits are harmful. Change the fewest lines that do the job; never
  rewrite a file or a chapter whole when a targeted edit will do. Do not
  overstep the boundary of the request, and stop as soon as it is done.
- Never change the live chat's prompts, sampling or served model without
  the user's approval of that exact change.
- No long GPU jobs on the Thor unless asked, and only in a time window the
  user gave.
- Draft changes to `jetson-thor/model-serving/thor-tigress-serve` outside the repo: thor-sync
  puts them on the Thor at once, and the next restart runs them.
- No Cloudflare. No AI attribution in commits.
