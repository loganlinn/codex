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

[tui.agent_commands.recent_messages]
description = "Show user and assistant messages from the last 10 turns"
interpreter = "jq"
args = ["-r", '.messages[] | "\(.role): \(.text)\n"', "{{context_file}}"]
message_turns = 10
```

`interpreter` is an executable name or path; `args` contains separate arguments.
`environment` overrides inherited environment variables. All three support
`{{variable}}` substitution, as does the optional `cwd` setting. Available variables:

| Context | Variables |
| --- | --- |
| Identity and ancestry | `thread_id`, `session_id`, `parent_thread_id`, `forked_from_id`, `project_id` |
| Workspace and Git | `cwd`, `git_branch`, `git_sha`, `git_origin_url`, `rollout_path` |
| Task | `title`, `preview`, `status`, `created_at`, `updated_at`, `ephemeral` |
| Agent | `agent_nickname`, `agent_role`, `agent_path`, `source`, `originator` |
| Model | `model`, `model_provider`, `reasoning_effort` |
| Runtime | `cli_version`, `tui_pid`, `context_file` |

Missing optional metadata expands to an empty string. Timestamps are Unix seconds;
booleans expand to `true` or `false`. Unknown or unclosed variables fail before
launching the command. Substituted values are not expanded a second time.

### Session context file

Every command receives `CODEX_AGENT_CONTEXT_FILE`, the absolute path of a temporary
JSON file. The same path is available as `{{context_file}}`; the environment variable
is reserved and cannot be overridden. This file has `schema_version = 1` and contains:

- `session`: all identity, workspace, task, agent, and model fields listed above,
  plus `cli_version`. Optional values are JSON `null` when unavailable.
- `captured_at`: when the context was assembled, in Unix seconds. Session metadata
  comes from the selected dashboard row, which may have been loaded earlier.
- `host.tui_pid`: the PID of the TUI launching the command. Sessions can share an
  app-server process, so there is no separate per-agent PID in this context.
- `host.terminal`: available values of `TERM`, `TERM_PROGRAM`,
  `TERM_PROGRAM_VERSION`, `KITTY_WINDOW_ID`, `TMUX_PANE`, `WEZTERM_PANE`,
  `WINDOWID`, `WT_SESSION`, and `ITERM_SESSION_ID`. These identify the launching
  TUI's terminal, not a selected session's original window. They are also inherited
  in the command's environment.
- `active_flags`: `waiting_on_approval` and/or `waiting_on_user_input` when reported.
  `session.status` is `not_loaded`, `idle`, `system_error`, or `active`.
- `source_details` and `environments`: structured app-server source and environment
  metadata, including runtime workspace roots when available. These objects use
  the app-server protocol's field names.
- `last_message_preview`: the dashboard's cached assistant preview, which may be
  shortened or stale; `null` when unavailable.
- `token_usage`: cached totals for this thread, or `null` when unavailable. Fields
  are `total_tokens`, `input_tokens`, `cached_input_tokens`,
  `cache_write_input_tokens`, `output_tokens`, and `reasoning_output_tokens`.
  These do not aggregate child agents or represent billing estimates.
- `messages`: `null` by default. Set `message_turns` to 1–100 to fetch user and
  assistant text from that many recent turns when the command runs. Entries appear
  in chronological order with `role`, `turn_id`, `item_id`, and full `text`.
  Assistant entries also include `phase`; user entries have
  `has_non_text_content` to indicate omitted attachments, skills, or mentions.
  Tool output and reasoning are excluded.
- `message_turns`: the actual number of turns fetched (0 when disabled).
  `messages_has_more` indicates older turns exist, and `messages_fetched_at` is
  the fetch time in Unix seconds. Both are `null` when message fetching is disabled.
  If history cannot be fetched within 15 seconds, the command fails before launch.
  An active turn's messages can be incomplete while the agent is streaming.

Git fields describe stored session metadata. Run Git in `{{cwd}}` for current
branch, changes, worktree, or review information. `rollout_path` is an optional
server-side storage path; it may be remote and is not guaranteed to be a transcript
file in every history mode.

The context file stays available for the command's lifetime and is removed after
exit, failure, or cancellation. If a command launches a detached consumer, copy the
file before returning. On Unix the temporary file is readable only by its owner.

### Execution

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
