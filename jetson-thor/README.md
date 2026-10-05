# Jetson Thor

The Thor is `192.168.0.83`, user `arpanpathak`. On yahboom it is just `thor`.

## Log in

```bash
ssh thor
```

No IP and no password. This comes from a block in `~/.ssh/config` and the key
`~/.ssh/id_ed25519`. To set the same up on another machine:

```bash
ssh-keygen -t ed25519 -N "" -f ~/.ssh/id_ed25519      # skip if the key exists
ssh-copy-id arpanpathak@192.168.0.83                  # asks for the password once
cat >> ~/.ssh/config <<'EOF'

Host thor
    HostName 192.168.0.83
    User arpanpathak
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 30
    ControlMaster auto
    ControlPath ~/.ssh/cm-%r@%h:%p
    ControlPersist 10m
EOF
```

`ControlMaster` keeps one connection open for 10 minutes, so later `ssh thor`
and `scp` calls start instantly.

Run one command without logging in: `ssh thor 'ollama ps'`.
Copy a file: `scp notes.md thor:` or `scp thor:Projects/x.log .`

## Keep projects in sync

`thor-sync` (in `~/.local/bin` on yahboom) copies folders to the same place
under the Thor's home, e.g. `~/Projects/openbatrangs` → `~/Projects/openbatrangs`.

```bash
thor-sync              # sync everything on the list
thor-sync add          # add the folder you are in
thor-sync rm           # take it off the list
thor-sync ls           # show the list
thor-sync on / off     # background sync, every change within a few seconds
```

On the list now: `thor-thunder-tigress-platform`, `openbatrangs`, `edgechat`.
Background sync is on. Git history is included, so `git` works on the Thor.
Skipped: anything `.gitignore` skips, plus `target/`, model files and the
chat export (edit `~/.config/thor-sync/exclude`). Sync goes one way, yahboom
to Thor: commit on yahboom, because a commit made on the Thor is overwritten.

## openBatarangs on the Thor

```bash
ssh thor
cd ~/Projects/some-project
openbatrangs                                  # interactive TUI
openbatrangs -m qwen3.6:27b --max-ctx 32768 "fix the failing test"
openbatrangs --read-only "explain this repo"  # no file writes or commands
openbatrangs doctor                           # check Ollama and the model
```

After changing openBatarangs on yahboom, rebuild it on the Thor:

```bash
ssh thor 'cd ~/Projects/openbatrangs && ~/.cargo/bin/cargo install --path .'
```

## Ollama

```bash
ollama list                          # downloaded models
ollama ps                            # loaded models; must say 100% GPU
ollama run qwen3.6:27b --verbose     # plain chat; "eval rate" is tokens/s
ollama stop qwen3.6:27b              # unload to free memory
```

## Measured

| Date | What | Result |
|---|---|---|
| 2026-10-04 | openBatarangs build on the Thor (release, 14 cores) | 19.5 s |
| 2026-10-04 | openBatarangs, qwen3.6:27b, read-only "list the crates" task | 6 steps, 21 s, correct |
