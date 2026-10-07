# What the context window is

People meet the phrase before they meet the idea, usually in a table:
"1M-token context". This chapter explains what a context window actually is,
why it is finite, and what part of it is the **KV cache**. It stays away from
the arithmetic; the measured numbers are in the chapter
[Memory, context and slots](ch20-memory-and-context.md).

<div class="covers">

This chapter covers

- the context window as one fixed budget per reply, not a memory
- why every message sends the whole conversation again
- what a token is, and what fills the window fastest
- what KV means, and why it grows with the conversation
- what happens when the window is full, and the four ways to live within it

</div>

## The desk, not a memory

<figure>
<img src="figures/context-window.svg" alt="The context window holds the system prompt, the whole conversation so far, any pasted text, and the reply being written, up to 1,048,576 tokens. The model keeps no memory between requests, so the browser re-sends the whole history. Four slots run four replies at once, each with its own window. A request longer than the window is refused.">
<figcaption><b>Figure 26.1</b> The window is a desk the model works at, cleared between requests.</figcaption>
</figure>

A **token** is a piece of a word: about four characters of English, a little
less for code. The **context window** is the most tokens one reply may hold. It
is a budget, and everything competes for it:

| Fills the window | Counts as |
|---|---|
| the system prompt, if the page sets one | tokens at the start of every request |
| every earlier message in the conversation | tokens, whole, every time |
| anything pasted: a log, a file, an error | tokens, in full |
| the reasoning, when **Think** is on | tokens, kept in the history after the answer |
| the reply being written | tokens, one at a time, as it is generated |

Nothing else fits. When the total reaches the limit, there is no room for the
next token.

## Why it is "one shot"

A language model does not remember your last message. It has no diary and no
session. Each request is the first request as far as the model is concerned.

So the browser does the remembering: it keeps the conversation and, with every
message, sends the **whole history** again. The server reads from the start
each time. That is also why the first word can take a moment on a long
conversation: the model is reading, not thinking.

The one shortcut is the **prompt cache**. A slot keeps the conversation it read
last. If the same conversation comes back to the same slot, the part already
read is not read again. It saves time; it is not memory. The cache is in
memory, and it is gone when the server restarts.

## Why the window is finite

A model that could look back at any number of tokens would still have to pay
for them, twice: in memory and in time.

### KV, in plain words

When the model writes the next word, each of its attention layers compares that
word with the words before it. To answer "which earlier words matter now?", the
layer keeps two small vectors for every earlier token:

- a **key**, what that token offers, and
- a **value**, what that token contributes.

<figure>
<img src="figures/kv-cache.svg" alt="For every earlier token the model keeps a key and a value. The query of the token being written is compared with every key before it, and the matching values are mixed into the next token. The KV cache is those keys and values, and it grows with the conversation.">
<figcaption><b>Figure 26.2</b> A key and a value per token, kept so they are not recomputed. This is the KV cache.</figcaption>
</figure>

Those keys and values, kept instead of recomputed, are the **KV cache**. It
grows with the conversation, one entry per token, per attention layer. Long
conversations are not just slower to read; they take memory that could have
been another slot.

The full arithmetic is in [Memory, context and slots](ch20-memory-and-context.md):

- This model keeps about **6 KiB per token**, so a million tokens is about
  **6 GiB per slot**.
- It is unusual because only **6 of its 52 layers** use attention this way. The
  rest keep one fixed-size state, however long the conversation. That hybrid
  design is the reason a million tokens fits on a 128 GB machine at all.

### Time

Reading time is the second bill. Before it writes anything, the model reads the
whole window. Measured on the Thor: about **1,050 to 1,100 tokens per second**.
A 100,000-token conversation is about a minute and a half before the first
word; a million tokens would be about sixteen. The prompt cache is what makes a
live conversation feel quick.

## What fills it fast

- **A pasted file.** A stack trace, a log, a whole source file. This is the
  fastest way to fill a window, and the model reads all of it.
- **Thinking.** With **Think** on, the reasoning is generated and then kept in
  the history, so it is paid for at every later message.
- **A long conversation.** Even without pastes, every reply stays in the
  history, and the history is sent whole each time.

## When it is full

The server refuses the request; it does not silently truncate and it does not
guess. A conversation that has grown past `CONTEXT` simply fails, and the page
shows the error. The fixes, in the order to try them:

1. **Start a new conversation** with **+**. Keep the old one in the browser if
   you still want it; the new one has a fresh window.
2. **Paste less.** Point at the file and the line instead of pasting all of it.
3. **Turn Think off** for routine questions, so reasoning is not added to the
   history.
4. **Raise `CONTEXT`** if you run the server, remembering that memory is
   `USERS × CONTEXT × 6 KiB`: a longer window means fewer slots at the same
   total memory, or more memory.

A fifth option exists on some clients, not on this one yet: a **sliding
window**, where the browser sends only the last N messages. It buys a longer
conversation at the cost of forgetting its start.

## Four slots, one machine

The window is per reply, not per person. Four replies run at once, each with
its own window, sharing one copy of the model weights. When all four are busy,
the next request waits in a queue. So the sentence to keep in mind is not
"four people" but "four replies at the same instant"; the window is what one of
those replies may hold.
