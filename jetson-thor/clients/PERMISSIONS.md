# Coding tool permissions with local models

We can let each coding tool in [`thor-clients.sh`](thor-clients.sh) edit files
and run commands without a prompt at every step. We keep the permission rules in
files and variables on our own machine. The same rules therefore work with any
local server that offers OpenAI's or Anthropic's API, such as llama.cpp, vLLM,
Ollama or TensorRT Edge-LLM.

| Tool | Where we set it | Default | Setting with no prompts |
|---|---|---|---|
| Claude Code | `~/.claude-thor/settings.json` | a prompt for each edit and command | commands in `permissions.allow` |
| OpenCode | `~/.config/opencode/opencode.json` | most tools allowed | `"permission": {"bash": "allow"}` |
| Deep Code | `~/.deepcode/settings.json` | everything allowed | `"defaultMode": "allowAll"` |
| DeepSeek Harness (`dsh`) | `DSH_PERMISSION_MODE` | a prompt for commands, writes inside the project | `danger-full-access` |
| openbatrangs | command-line flags | everything allowed | no flag |

## Claude Code

We choose one of five permission modes. During a session, we switch modes with
Shift+Tab, and we set the starting mode with `permissions.defaultMode`.

| Mode | With a local model |
|---|---|
| `default` | a prompt for each edit and for each command outside the allow list |
| `acceptEdits` | edits allowed, a prompt for each command outside the allow list |
| `plan` | reading and planning only |
| `auto` | every command blocked |
| `bypassPermissions` | everything allowed |

We avoid auto mode with a local model. In auto mode, Anthropic's safety
classifier must approve each command before it runs. A local server has no
classifier, so every command in auto mode fails with "classifier unavailable".
An allow list gives routine commands the same speed and needs no classifier.

When we start `claude-thor` for the first time, it copies
[`claude-thor-settings.json`](claude-thor-settings.json) to
`~/.claude-thor/settings.json`, unless that file already exists. The settings
begin with these rules.

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
When a command matches both an allow rule and a deny rule, the deny rule
applies. A command that matches neither needs our approval at a prompt. To
allow a command in one project only, we add the rule to that project's
`.claude/settings.json`.

To turn every prompt off, we start `claude-thor --dangerously-skip-permissions`.
Destructive commands then run without a prompt too, so we use this flag only
inside a container or a throwaway checkout.

## OpenCode

We set OpenCode's permissions in the `permission` block of
`~/.config/opencode/opencode.json`. Each key holds one of three values: `allow`,
`ask` or `deny`. On the Thor, we use these values.

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

The `external_directory` key controls access to files outside the project. The
`doom_loop` key applies when the model repeats the same tool call several times
in a row, and with `deny`, the run ends there. We deny `webfetch`, so the model
cannot download web pages during a run.

For finer control, we can give `bash` a list of command patterns in place of
one value, as described in the
[OpenCode permissions documentation](https://opencode.ai/docs/permissions/).
With `opencode --auto`, every action that is not denied runs without a prompt.

## Deep Code

Deep Code's permissions work by scope. Each tool call belongs to one or more of
these scopes.

- Reading: `read-in-cwd`, `read-in-tmp`, `read-out-cwd`
- Writing: `write-in-cwd`, `write-in-tmp`, `write-out-cwd`
- Deleting: `delete-in-cwd`, `delete-out-cwd`
- Git: `query-git-log`, `mutate-git-log`
- Other: `network`, `mcp`

A command whose effects are unclear belongs to the `unknown` scope and always
needs our approval. By default, `defaultMode` is `allowAll`, so every other
scope runs without a prompt. With the settings below, work inside the project
runs without a prompt, work outside it needs our approval, and deletes outside
it are refused.

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

When a call belongs to several scopes, `deny` applies first, then `ask`, then
`allow`. When we answer "Yes, and always allow" at a prompt, the scope is added
to the project's `.deepcode/settings.json`.

`deepcode-thor` passes its model settings with `-s`, which replaces the
project's `.deepcode/settings.json`. We therefore put permissions for the Thor's
models in `~/.deepcode/settings.json`.

## DeepSeek Harness (`dsh`)

We set the mode of DeepSeek Harness with the `DSH_PERMISSION_MODE` variable.

| `DSH_PERMISSION_MODE` | Writes allowed | Prompt before commands |
|---|---|---|
| `read-only` | none | yes |
| `workspace-write` (default) | inside the project | yes |
| `danger-full-access` | anywhere | no |

To run `deepseek-thor` without prompts, we start it with
`DSH_PERMISSION_MODE=danger-full-access deepseek-thor`. In the other two modes,
shell commands run in a sandbox with the write limits shown in the table.

## openbatrangs

By default, openbatrangs edits files and runs commands without a prompt. We can
restrict it with two flags.

| Flag | Effect |
|---|---|
| `--confirm` | a prompt before each file write and each shell command |
| `--read-only` | reading and searching only, with no writes and no commands |
