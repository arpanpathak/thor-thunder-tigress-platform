# Coding tool permissions with local models

Every coding tool in [`thor-clients.sh`](thor-clients.sh) shows a prompt before
a file edit or a shell command, unless its settings allow that action. In this
guide, you'll find where each tool's settings are stored, what is allowed by
default, and a setting for building, testing and committing without a prompt at
every step. Permission settings are read on your own machine, so they apply
with any local server that offers OpenAI's or Anthropic's API, such as
llama.cpp, vLLM, Ollama or TensorRT Edge-LLM.

| Tool | Settings | Default | For no prompts |
|---|---|---|---|
| Claude Code | `~/.claude-thor/settings.json` | prompt for each edit and command | commands listed in `permissions.allow` |
| OpenCode | `~/.config/opencode/opencode.json` | most tools allowed | `"permission": {"bash": "allow"}` |
| Deep Code | `~/.deepcode/settings.json` | everything allowed | `"defaultMode": "allowAll"` |
| DeepSeek Harness (`dsh`) | `DSH_PERMISSION_MODE` | prompts, writes only inside the project | `DSH_PERMISSION_MODE=danger-full-access` |
| openbatrangs | command-line flags | everything allowed | no flag needed |

## Claude Code

You can pick one of five permission modes. Shift+Tab switches the mode during a
session, and `permissions.defaultMode` sets the mode at start.

| Mode | With a local model |
|---|---|
| `default` | a prompt for each edit, and for each command outside the allow list |
| `acceptEdits` | edits allowed, a prompt for each command outside the allow list |
| `plan` | reading and planning only, with no changes |
| `auto` | every command blocked (explained below) |
| `bypassPermissions` | everything allowed |

In auto mode, each command goes to Anthropic's safety classifier first, and only
commands it approves are run. A local server has no classifier. With
`claude-thor`, every command in auto mode therefore fails with "classifier
unavailable". An allow list gives you the same speed for routine work with no
classifier involved.

On the first start of `claude-thor`, the file
[`claude-thor-settings.json`](claude-thor-settings.json) is copied to
`~/.claude-thor/settings.json`. Its first lines are these.

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
Deny rules have priority over allow rules. For a command that matches neither,
you'll see a prompt. To allow a command in one project only, add the rule to
that project's `.claude/settings.json`.

To turn every prompt off, start with `claude-thor --dangerously-skip-permissions`.
In that mode, a destructive command runs with no prompt, so use it only inside a
container or a throwaway checkout.

## OpenCode

In `opencode.json`, the `permission` block holds one value for each tool:
`allow`, `ask` or `deny`. For the Thor's OpenCode setup, we use these values.

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

The `external_directory` key applies to files outside the project. With
`doom_loop` set to `deny`, a run ends when the same tool call repeats several
times in a row. With `webfetch` set to `deny`, no web pages are downloaded during
a run.

For finer control, you can give `bash` a set of command patterns instead of a
single value. The [OpenCode permissions documentation](https://opencode.ai/docs/permissions/)
explains how patterns are matched. With `opencode --auto`, every action not set
to `deny` is approved.

## Deep Code

For each tool call, Deep Code assigns a scope and then applies the `permissions`
block in `~/.deepcode/settings.json`. These are the scopes.

- Reading: `read-in-cwd`, `read-in-tmp`, `read-out-cwd`
- Writing: `write-in-cwd`, `write-in-tmp`, `write-out-cwd`
- Deleting: `delete-in-cwd`, `delete-out-cwd`
- Git: `query-git-log`, `mutate-git-log`
- Other: `network`, `mcp`

A command with unclear effects is assigned the `unknown` scope, and you always
get a prompt for it. By default, with `"defaultMode": "allowAll"`, every other
scope is allowed. With the setting below, work inside the project needs no
prompt, and work outside it needs one.

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

The `deny` list has priority over `ask`, and `ask` has priority over `allow`.
If you choose "Yes, and always allow" at a prompt, the scope is added to the
project's `.deepcode/settings.json`.

The `deepcode-thor` command passes its model settings with `-s`, in place of
the project's `.deepcode/settings.json`. For the Thor's models, put the
permissions in `~/.deepcode/settings.json`.

## DeepSeek Harness (`dsh`)

In `dsh`, the `DSH_PERMISSION_MODE` variable sets the mode.

| `DSH_PERMISSION_MODE` | Writes allowed | Prompt before commands |
|---|---|---|
| `read-only` | none | yes |
| `workspace-write` (default) | inside the project | yes |
| `danger-full-access` | anywhere | no |

To start `deepseek-thor` without prompts, run
`DSH_PERMISSION_MODE=danger-full-access deepseek-thor`. In the other two modes,
shell commands run in a sandbox with the write limits shown in the table.

## openbatrangs

In openbatrangs, edits and commands are allowed by default. Two flags restrict
them.

| Flag | Effect |
|---|---|
| `--confirm` | a prompt before each file write and each shell command |
| `--read-only` | reading and searching only, with no writes and no commands |

## Safety checks on a vendor's servers

In Claude Code's auto mode, the check on each command runs on Anthropic's
servers. With a local server, that check is unreachable, and every command in
the mode is blocked. Allow and deny lists are checked on your own machine, so
they work with any server.
