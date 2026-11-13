# Coding tool permissions with local models

A coding tool asks before it edits a file or runs a command, unless its
settings allow the action. This guide lists, for each tool in
[`thor-clients.sh`](thor-clients.sh), where those settings live, what the tool
does by default, and a setting that lets it build, test and commit without a
prompt for every step. The same settings work with any local server that
offers OpenAI's or Anthropic's API, such as llama.cpp, vLLM, Ollama or
TensorRT Edge-LLM.

| Tool | Settings | Default | Runs commands without asking |
|---|---|---|---|
| Claude Code | `~/.claude-thor/settings.json` | asks for each edit and command | allow list in `permissions.allow` |
| OpenCode | `~/.config/opencode/opencode.json` | runs most tools without asking | `"permission": {"bash": "allow"}` |
| Deep Code | `~/.deepcode/settings.json` | runs everything without asking | `"defaultMode": "allowAll"` |
| DeepSeek Harness (`dsh`) | `DSH_PERMISSION_MODE` | asks, and writes only inside the project | `DSH_PERMISSION_MODE=danger-full-access` |
| openbatrangs | command-line flags | runs everything without asking | no flag |

## Claude Code

Claude Code has five permission modes. Shift+Tab switches between them in a
session, and `permissions.defaultMode` sets the mode it starts in.

| Mode | Behavior with a local model |
|---|---|
| `default` | asks before each edit and each command outside the allow list |
| `acceptEdits` | edits files without asking, asks for commands outside the allow list |
| `plan` | reads and plans, changes nothing |
| `auto` | blocks every command (see below) |
| `bypassPermissions` | runs everything without asking |

Auto mode sends each command to Anthropic's safety classifier and runs it only
when the classifier approves. A local server has no classifier, so every
command fails with "classifier unavailable". The allow list gives the same
speed for routine work without the classifier.

On its first start, `claude-thor` copies
[`claude-thor-settings.json`](claude-thor-settings.json) to
`~/.claude-thor/settings.json`. Its first lines look like this:

```json
{
  "permissions": {
    "defaultMode": "acceptEdits",
    "allow": [
      "Bash(cargo:*)",
      "Bash(git commit:*)",
      "Bash(gh pr:*)"
    ],
    "deny": [
      "Bash(git push --force:*)",
      "Bash(git reset --hard:*)"
    ]
  }
}
```

A rule such as `Bash(cargo:*)` matches every command that starts with `cargo`.
A deny rule wins over an allow rule, and a command that matches neither gets a
prompt. To add a command for one project only, put the rule in that project's
`.claude/settings.json`.

To skip every prompt, start with `claude-thor --dangerously-skip-permissions`.
Nothing then stops a destructive command, so use it only inside a container or
a throwaway checkout.

## OpenCode

OpenCode reads a `permission` block from `opencode.json`. Each tool gets
`allow`, `ask` or `deny`. The Thor's OpenCode setup uses these values:

```json
{
  "permission": {
    "edit": "allow",
    "bash": "allow",
    "read": "allow",
    "glob": "allow",
    "grep": "allow",
    "list": "allow",
    "external_directory": "allow",
    "webfetch": "deny",
    "doom_loop": "deny"
  }
}
```

`external_directory` covers files outside the project. `doom_loop` covers a
model that repeats the same tool call several times in a row, and denying it
stops such a loop. `webfetch` is denied, so the model cannot download pages
while it works.

For finer control, `bash` also takes command patterns. The
[OpenCode permissions documentation](https://opencode.ai/docs/permissions/)
describes how they match.

```json
{
  "permission": {
    "bash": {
      "*": "allow",
      "git push *": "ask",
      "rm -rf *": "deny"
    }
  }
}
```

`opencode --auto` approves every action that the settings do not deny.

## Deep Code

Deep Code sorts each action into a scope, then allows, denies or asks
according to `permissions` in `~/.deepcode/settings.json`. The scopes are
`read-in-cwd`, `read-in-tmp`, `read-out-cwd`, `write-in-cwd`, `write-in-tmp`,
`write-out-cwd`, `delete-in-cwd`, `delete-out-cwd`, `query-git-log`,
`mutate-git-log`, `network` and `mcp`. A command whose effect the model cannot
classify falls into `unknown`, which always asks.

The default, `"defaultMode": "allowAll"`, runs everything else without asking.
This setting keeps that speed inside the project and asks before anything
outside it:

```json
{
  "permissions": {
    "defaultMode": "askAll",
    "allow": [
      "read-in-cwd", "write-in-cwd", "delete-in-cwd",
      "read-in-tmp", "write-in-tmp",
      "query-git-log", "mutate-git-log"
    ],
    "deny": ["delete-out-cwd"]
  }
}
```

A deny wins over an ask, and an ask wins over an allow. "Yes, and always
allow" at a prompt adds the scope to the project's `.deepcode/settings.json`.
`deepcode-thor` passes its model settings with `-s`, which takes the place of
the project's `.deepcode/settings.json`. Permissions for the Thor's models
therefore go in `~/.deepcode/settings.json`.

## DeepSeek Harness (`dsh`)

`dsh` takes its mode from the `DSH_PERMISSION_MODE` variable.

| `DSH_PERMISSION_MODE` | Files it may write | Asks before commands |
|---|---|---|
| `read-only` | none | yes |
| `workspace-write` (default) | the project | yes |
| `danger-full-access` | anywhere | no |

To start `deepseek-thor` without prompts, run
`DSH_PERMISSION_MODE=danger-full-access deepseek-thor`. In the other two modes,
`dsh` also runs shell commands in a sandbox that limits where they can write.

## openbatrangs

openbatrangs runs edits and commands without asking. Two flags make it
stricter:

| Flag | Behavior |
|---|---|
| `--confirm` | asks before each file write and each shell command |
| `--read-only` | reads and searches only, no writes and no commands |

## Hosted classifiers and local models

Some tools check commands with a model on the vendor's servers. Claude Code's
auto mode is one example. A tool pointed at a local server cannot reach that
check, so its modes that depend on it block everything. Allow and deny lists
are evaluated on your machine and work with any server.
