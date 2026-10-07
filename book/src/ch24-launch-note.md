# Launch note: what we store, and what we do not

This chapter is the note that goes out with the public chat. It is short, and
it is meant to be checked against the code, not taken on trust. Everything it
claims is in this repository.

The promise in one line: **your conversations stay in your browser; the only
thing we keep is the name and the email address you gave us to get a key.**

<figure>
<img src="figures/what-is-stored.svg" alt="Three columns: your browser, the Thor's memory and the Thor's disk. The browser holds the conversations, the settings and the key. The Thor's memory holds the conversation it is answering, in a slot cache, gone on restart. The Thor's disk holds one sealed file with a name, an email, a status and a key per person, and the single key the servers use between themselves. Message text is never written down.">
<figcaption><b>Figure 24.1</b> Everything the system keeps, and where it keeps it.</figcaption>
</figure>

## What is stored

| Where | What | For how long |
|---|---|---|
| your browser | the conversation: every message, code block and thought; the settings; the key you were given | until you clear site data or press **+** |
| the Thor's memory | the conversation a slot is answering, so the next message on that slot is not read twice | until the model server restarts |
| the Thor's disk | one file, `~/.config/thor-chat/keyring`: your name, your email, whether you are active, your key, the time | until you ask to be removed |

That is the whole list. There is no third column.

## What is not stored

- **Message text.** Not in a database, not in a log, not in a file. The server
  has nowhere to put it: the only file it writes is the keyring, and a keyring
  record has no field for a message. You can read the schema in
  `crates/thor-tigress-keyring/src/store.rs`.
- **Cookies from other sites, analytics, advertising.** The page sets no
  cookie and loads nothing from another domain.
- **A password, a phone number, a login.** There is no account. A key is not a
  login; it is a long random string you can throw away.
- **Your IP address in a database.** Requests are answered and forgotten. The
  operating system keeps a short journal for its own logs, the same as any
  service on the machine.

## Why there are no rate limits

By design, not by accident. One Jetson Thor serves four replies at once. When
all four are busy, the next request waits; nothing is refused for being busy.
There is no daily allowance, no per-person quota and no token accounting,
because the point of a small public beta is to use the thing hard and find
where it breaks.

That trade has a price, and it is worth saying plainly: **one person with a key
can keep the Thor busy.** The limits are social, not technical. If that becomes
a problem, the answer is a key revocation, not a rate limiter.

### If everyone sends at once

Nothing breaks, and nobody is locked out for being busy. The four slots fill,
the queue grows, and answers take longer; a fifth request waits its turn. Only
two things are refused outright: a request without a valid key, and a request
longer than the model's context window. There is no automatic ban, no
per-person pacing and no penalty for a heavy day.

The machine also runs warm while it works. On a cold day that is not a
complaint: a Thor holding four replies is a small space heater that answers
questions, which is an unplanned perk of a service that lives in a home rather
than a data centre.

## Keys, and leaks

Every person gets their own key, so access is given and taken one person at a
time. Keys leak; the design assumes it.

- **One command stops everyone at once.** `thor-tigress-serve keyring
  revoke-all` marks every active key revoked. The chat server reads the file
  when it changes, so the next request is refused: no restart, no downtime for
  anyone else, and the records stay so the people you trust can be approved
  again.
- **One command stops one person.** `thor-tigress-serve keyring revoke
  EMAIL` or `revoke KEY`.
- **The keys never reach the model server.** `thor-tigress-agent` checks your
  personal key, then talks to llama-server with its own single key. A key you
  hold cannot be used on the model server directly, and revoking it does not
  touch the others.

The full mechanics, and the cryptography under them, are in the next chapter:
[Keys, and the cryptography under them](ch25-keyring-crypto.md).

## The limits of this promise

An honest note names what it does not cover.

- **The operator can read the keyring.** Whoever holds the Thor's user account
  and the passphrase file can open the file and see the names, emails and keys.
  Encryption here protects a copy of the file that is taken away, not the
  machine it sits on.
- **The registration form is open.** Anyone can post a name and an email; that
  is the point. It can be spammed. A person approves each request by hand, so
  spam fills the waiting list and nothing more.
- **A key is a password.** Anyone who has one can use the chat as you, until it
  is revoked. Do not paste it into a chat or a screenshot.
- **The Thor is one machine.** It can be offline, busy, or out of memory. When
  it is down, the forwarding page still loads and the chat shows a red dot.

## How to get a key

Give your name and email on the invite screen, then send a message so we know
it is a person asking:

- [DM on LinkedIn](https://www.linkedin.com/in/arpan-pathak-272341424/)
- [DM on X](https://x.com/arpanpathak1996)

You will get a key back, once. It is a 48-character string; keep it somewhere
private.

## The small print, in full

- License and source: [the repository](https://github.com/arpanpathak/thor-thunder-tigress-platform).
- The book's [Security](ch10-security.md) chapter lists what is exposed and
  what protects it.
- [Memory, context and slots](ch20-memory-and-context.md) explains what the
  machine is doing while you chat.
- [What the context window is](ch26-context-window.md) explains, without maths,
  why a conversation has a limit at all.
