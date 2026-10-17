//! Versioned command input from dashboard metadata and optional recent messages.
//! The scalar session fields also form the template vocabulary.

use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::SessionSource;
use codex_app_server_protocol::SortDirection;
use codex_app_server_protocol::Thread;
use codex_app_server_protocol::ThreadActiveFlag;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::ThreadStatus;
use codex_app_server_protocol::ThreadTurnsListParams;
use codex_app_server_protocol::ThreadTurnsListResponse;
use codex_app_server_protocol::TokenUsageBreakdown;
use codex_app_server_protocol::TurnItemsView;
use codex_app_server_protocol::UserInput;
use codex_protocol::protocol::SubAgentSource;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

pub(super) struct CommandContext {
    pub(super) document: Value,
    pub(super) variables: BTreeMap<String, String>,
}

impl CommandContext {
    pub(super) fn new(
        thread: &Thread,
        last_message_preview: Option<&str>,
        token_usage: Option<&TokenUsageBreakdown>,
    ) -> Self {
        let (status, flags) = match &thread.status {
            ThreadStatus::NotLoaded => ("not_loaded", &[][..]),
            ThreadStatus::Idle => ("idle", &[][..]),
            ThreadStatus::SystemError => ("system_error", &[][..]),
            ThreadStatus::Active { active_flags } => ("active", active_flags.as_slice()),
        };
        let active_flags = flags
            .iter()
            .map(|flag| match flag {
                ThreadActiveFlag::WaitingOnApproval => "waiting_on_approval",
                ThreadActiveFlag::WaitingOnUserInput => "waiting_on_user_input",
            })
            .collect::<Vec<_>>();
        let source = match &thread.source {
            SessionSource::Cli => "cli",
            SessionSource::VsCode => "vscode",
            SessionSource::Exec => "exec",
            SessionSource::AppServer => "app_server",
            SessionSource::Custom(_) => "custom",
            SessionSource::SubAgent(_) => "subagent",
            SessionSource::Unknown => "unknown",
        };
        // Older metadata may carry the parent and agent identity only in the source.
        let (parent_id, nickname, role, agent_path) = match &thread.source {
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                agent_nickname,
                agent_role,
                agent_path,
                ..
            }) => (
                Some(parent_thread_id.to_string()),
                agent_nickname.as_deref(),
                agent_role.as_deref(),
                agent_path.as_ref().map(ToString::to_string),
            ),
            _ => (None, None, None, None),
        };
        let session = BTreeMap::from([
            ("thread_id", json!(thread.id)),
            ("session_id", json!(thread.session_id)),
            (
                "parent_thread_id",
                json!(thread.parent_thread_id.as_deref().or(parent_id.as_deref())),
            ),
            ("forked_from_id", json!(thread.forked_from_id)),
            ("project_id", json!(thread.project_id)),
            ("cwd", json!(thread.cwd)),
            (
                "title",
                json!(thread.name.as_deref().unwrap_or(&thread.preview)),
            ),
            ("preview", json!(thread.preview)),
            ("model", json!(thread.model)),
            ("model_provider", json!(thread.model_provider)),
            (
                "reasoning_effort",
                json!(
                    thread
                        .reasoning_effort
                        .as_ref()
                        .map(|effort| effort.as_str())
                ),
            ),
            ("status", json!(status)),
            ("source", json!(source)),
            ("originator", json!(thread.originator)),
            (
                "agent_nickname",
                json!(thread.agent_nickname.as_deref().or(nickname)),
            ),
            ("agent_role", json!(thread.agent_role.as_deref().or(role))),
            ("agent_path", json!(agent_path)),
            ("rollout_path", json!(thread.path)),
            (
                "git_branch",
                json!(
                    thread
                        .git_info
                        .as_ref()
                        .and_then(|git| git.branch.as_deref())
                ),
            ),
            (
                "git_sha",
                json!(thread.git_info.as_ref().and_then(|git| git.sha.as_deref())),
            ),
            (
                "git_origin_url",
                json!(
                    thread
                        .git_info
                        .as_ref()
                        .and_then(|git| git.origin_url.as_deref())
                ),
            ),
            ("created_at", json!(thread.created_at)),
            ("updated_at", json!(thread.updated_at)),
            ("cli_version", json!(thread.cli_version)),
            ("ephemeral", json!(thread.ephemeral)),
        ]);
        let mut variables: BTreeMap<String, String> = session
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    Value::Null => String::new(),
                    Value::String(value) => value.clone(),
                    _ => value.to_string(),
                };
                (key.to_string(), value)
            })
            .collect();
        let tui_pid = std::process::id();
        variables.insert("tui_pid".to_string(), tui_pid.to_string());
        // Only identify the launching terminal. A selected session can be remote or
        // historical and need not have its own OS process or terminal window.
        let terminal = [
            "TERM",
            "TERM_PROGRAM",
            "TERM_PROGRAM_VERSION",
            "KITTY_WINDOW_ID",
            "TMUX_PANE",
            "WEZTERM_PANE",
            "WINDOWID",
            "WT_SESSION",
            "ITERM_SESSION_ID",
        ]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| (key, value)))
        .collect::<BTreeMap<_, _>>();
        let document = json!({
            "schema_version": 1,
            "captured_at": chrono::Utc::now().timestamp(),
            "host": { "tui_pid": tui_pid, "terminal": terminal },
            "session": session,
            "active_flags": active_flags,
            // Preserve structured source details without making them shell arguments.
            "source_details": thread.source,
            "environments": thread.environments,
            "last_message_preview": last_message_preview,
            "messages": null,
            "messages_has_more": null,
            "message_turns": 0,
            "messages_fetched_at": null,
            "token_usage": token_usage.map(|usage| json!({
                "total_tokens": usage.total_tokens,
                "input_tokens": usage.input_tokens,
                "cached_input_tokens": usage.cached_input_tokens,
                "cache_write_input_tokens": usage.cache_write_input_tokens,
                "output_tokens": usage.output_tokens,
                "reasoning_output_tokens": usage.reasoning_output_tokens,
            })),
        });
        Self {
            document,
            variables,
        }
    }

    pub(super) async fn load_messages(
        &mut self,
        request_handle: &AppServerRequestHandle,
        thread_id: &str,
        message_turns: u32,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            message_turns <= 100,
            "message_turns must be between 0 and 100"
        );
        if message_turns == 0 {
            return Ok(());
        }
        let response = tokio::time::timeout(
            Duration::from_secs(15),
            request_handle.request_typed::<ThreadTurnsListResponse>(
                ClientRequest::ThreadTurnsList {
                    request_id: RequestId::String(Uuid::new_v4().to_string()),
                    params: ThreadTurnsListParams {
                        thread_id: thread_id.to_string(),
                        cursor: None,
                        limit: Some(message_turns),
                        sort_direction: Some(SortDirection::Desc),
                        items_view: Some(TurnItemsView::Full),
                    },
                },
            ),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out fetching custom command messages"))?
        .map_err(|error| anyhow::anyhow!("cannot fetch custom command messages: {error}"))?;
        self.set_messages(response);
        Ok(())
    }

    fn set_messages(&mut self, response: ThreadTurnsListResponse) {
        self.document["message_turns"] = json!(response.data.len());
        self.document["messages_has_more"] = json!(response.next_cursor.is_some());
        self.document["messages_fetched_at"] = json!(chrono::Utc::now().timestamp());
        let mut messages = Vec::new();
        // The page is newest turn first; items within each turn are chronological.
        for turn in response.data.into_iter().rev() {
            for item in turn.items {
                let message = match item {
                    ThreadItem::UserMessage { id, content, .. } => {
                        let text = content
                            .iter()
                            .filter_map(|input| match input {
                                UserInput::Text { text, .. } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        json!({
                            "role": "user", "turn_id": turn.id, "item_id": id, "text": text,
                            "has_non_text_content": content.iter().any(|input| !matches!(input, UserInput::Text { .. })),
                        })
                    }
                    ThreadItem::AgentMessage {
                        id, text, phase, ..
                    } => json!({
                        "role": "assistant", "turn_id": turn.id, "item_id": id,
                        "text": text, "phase": phase,
                    }),
                    _ => continue,
                };
                messages.push(message);
            }
        }
        self.document["messages"] = json!(messages);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn messages_are_chronological_full_text_without_tool_or_reasoning_items() {
        let text = "line one\n'quotes' $HOME {{cwd}} ".repeat(100);
        let response = serde_json::from_value(json!({
            "data": [
                {
                    "id": "newer", "status": "completed", "items": [
                        {"type": "userMessage", "id": "u2", "content": [
                            {"type": "text", "text": "part one"},
                            {"type": "text", "text": "part two"},
                            {"type": "mention", "name": "file", "path": "/tmp/file"}
                        ]},
                        {"type": "reasoning", "id": "r2", "summary": ["private"], "content": []},
                        {"type": "agentMessage", "id": "a2", "text": text, "phase": "final_answer"}
                    ]
                },
                {
                    "id": "older", "status": "completed", "items": [
                        {"type": "userMessage", "id": "u1", "content": [{"type": "text", "text": "first"}]},
                        {"type": "plan", "id": "p1", "text": "not a message"},
                        {"type": "agentMessage", "id": "a1", "text": "second"}
                    ]
                }
            ],
            "nextCursor": "more-history"
        })).unwrap();
        let mut context = CommandContext {
            document: json!({}),
            variables: BTreeMap::new(),
        };
        context.set_messages(response);
        let messages = context.document["messages"].as_array().unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|message| message["item_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["u1", "a1", "u2", "a2"]
        );
        assert_eq!(messages[0]["has_non_text_content"], false);
        assert_eq!(messages[2]["has_non_text_content"], true);
        assert_eq!(messages[2]["text"], "part one\npart two");
        assert_eq!(messages[3]["text"], text);
        assert_eq!(messages[3]["phase"], "final_answer");
        assert_eq!(context.document["messages_has_more"], true);
        assert_eq!(context.document["message_turns"], 2);
    }
}
