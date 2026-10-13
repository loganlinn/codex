# Configuration

For basic configuration instructions, see [this documentation](https://developers.openai.com/codex/config-basic).

For advanced configuration instructions, see [this documentation](https://developers.openai.com/codex/config-advanced).

For a full configuration reference, see [this documentation](https://developers.openai.com/codex/config-reference).

## Agent command center commands

Define named commands in `~/.codex/config.toml` (or `$CODEX_HOME/config.toml`).
In the agents command center, select a task and press `c` to open the searchable
command menu. The shortcut can be changed with `tui.keymap.agents.commands`.

```toml
[tui.agent_commands.open_editor]
description = "Open the selected task's checkout in VS Code"
interpreter = "code"
args = ["{{cwd}}"]

[tui.agent_commands.inspect]
description = "Inspect the selected task's checkout"
interpreter = "/bin/sh"
args = ["-c", 'printf "%s\\n" "$TASK_TITLE"; git status --short']

[tui.agent_commands.inspect.environment]
TASK_ID = "{{thread_id}}"
TASK_TITLE = "{{title}}"
```

`interpreter` is an executable name or path; `args` contains separate arguments.
`environment` overrides inherited environment variables. All three support
`{{variable}}` substitution. Available variables are `thread_id`, `session_id`,
`cwd`, `title`, `model`, and `git_branch`. Missing model or branch metadata expands
to an empty string. Unknown or unclosed variables fail before launching the command.
Substituted values are not expanded a second time.

Commands run on the machine hosting the TUI, including when connected to a remote
app server. The working directory defaults to the selected task's `cwd`; set the
optional `cwd` template to override it. Relative directories resolve against the
task directory. When the task directory exists only remotely, specify a local
absolute `cwd` and invoke your remote tooling explicitly.

Commands run in the background without terminal input. The dashboard stays
usable, and the command menu offers cancellation and the last command's output.
Only one custom command runs at a time. Completion displays stdout, stderr, and
exit status when the dashboard is active; otherwise, open the command menu to
view the result. Output retains up to 64 KiB per stream. Commands execute directly
with your local permissions and environment, outside agent turns and the agent
sandbox. Configuration changes take effect after restarting the TUI.

No shell quoting or expansion is added. If you invoke a shell with `-c`, pass
session data through environment variables or positional arguments, as above,
and quote those values in the script. Substituting task text directly into shell
source makes it executable shell syntax.

## Lifecycle hooks

Admins can set top-level `allow_managed_hooks_only = true` in
`requirements.toml` to ignore user, project, and session hook configs while
still allowing managed hooks from requirements and managed config layers. This
setting is only supported in `requirements.toml`; putting it in `config.toml`
does not enable managed-hooks-only mode.
