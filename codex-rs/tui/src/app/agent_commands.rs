//! Explicit, client-local commands use a snapshot of the selected dashboard task.
//! Execution is independent of agent turns; results never enter a model conversation.

mod process;

use super::*;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use codex_app_server_protocol::Thread;

#[derive(Default)]
pub(super) struct AgentCommandState {
    running: Option<(Uuid, String, tokio::task::AbortHandle)>,
    result: Option<(String, String)>,
}

impl Drop for AgentCommandState {
    fn drop(&mut self) {
        if let Some((_, _, task)) = self.running.take() {
            task.abort();
        }
    }
}

impl App {
    pub(super) fn open_agent_commands(&mut self, thread: Option<Thread>) {
        let mut items = Vec::new();
        if let Some((request_id, name, _)) = &self.agents_overview.commands.running {
            let request_id = *request_id;
            items.push(SelectionItem {
                name: format!("Cancel {name}"),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::CancelAgentCommand { request_id });
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }
        if self.agents_overview.commands.result.is_some() {
            items.push(SelectionItem {
                name: "Show last command output".to_string(),
                actions: vec![Box::new(|tx| tx.send(AppEvent::ShowAgentCommandOutput))],
                dismiss_on_select: true,
                ..Default::default()
            });
        }
        for (name, command) in &self.local_settings.tui.agent_commands {
            let Some(thread) = &thread else { break };
            let name = name.clone();
            let thread = thread.clone();
            items.push(SelectionItem {
                name: name.clone(),
                description: command.description.clone(),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::RunAgentCommand {
                        name: name.clone(),
                        thread: Box::new(thread.clone()),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }
        self.chat_widget.show_selection_view(SelectionViewParams {
            title: Some("Custom commands".to_string()),
            subtitle: Some(if items.is_empty() {
                if self.local_settings.tui.agent_commands.is_empty() {
                    "Add commands under [tui.agent_commands.<name>] in config.toml.".to_string()
                } else {
                    "Select a task to run a custom command.".to_string()
                }
            } else {
                thread.as_ref().map_or_else(
                    || "Local commands".to_string(),
                    |thread| {
                        format!(
                            "Run locally for {}",
                            thread.name.as_deref().unwrap_or(&thread.id)
                        )
                    },
                )
            }),
            allow_input_when_disconnected: true,
            items,
            is_searchable: true,
            ..SelectionViewParams::picker()
        });
    }

    pub(super) fn run_agent_command(&mut self, name: String, thread: Thread) {
        if self.agents_overview.commands.running.is_some() {
            self.add_agents_overview_error(
                "A custom command is already running. Use the command menu to cancel it."
                    .to_string(),
            );
            return;
        }
        let Some(command) = self.local_settings.tui.agent_commands.get(&name) else {
            self.add_agents_overview_error(format!(
                "Custom command {name:?} is no longer configured."
            ));
            return;
        };
        let command = match process::prepare(command, &thread) {
            Ok(command) => command,
            Err(error) => {
                self.add_agents_overview_error(format!("Cannot run {name}: {error}"));
                return;
            }
        };
        let request_id = Uuid::new_v4();
        let tx = self.app_event_tx.clone();
        let task = tokio::spawn(async move {
            let result = process::run(command)
                .await
                .map_err(|error| error.to_string());
            tx.send(AppEvent::AgentCommandFinished { request_id, result });
        });
        self.set_agent_command_notice(Some(format!("Running {name}…")));
        self.agents_overview.commands.running = Some((request_id, name, task.abort_handle()));
    }

    pub(super) fn cancel_agent_command(&mut self, request_id: Uuid) {
        if self
            .agents_overview
            .commands
            .running
            .as_ref()
            .map(|(id, _, _)| *id)
            != Some(request_id)
        {
            return;
        }
        if let Some((_, name, task)) = self.agents_overview.commands.running.take() {
            task.abort();
            self.set_agent_command_notice(Some(format!("Canceled {name}")));
        }
    }

    pub(super) fn finish_agent_command(
        &mut self,
        tui: &mut tui::Tui,
        request_id: Uuid,
        result: Result<String, String>,
    ) {
        if self
            .agents_overview
            .commands
            .running
            .as_ref()
            .map(|(id, _, _)| *id)
            != Some(request_id)
        {
            return;
        }
        if let Some((_, name, _)) = self.agents_overview.commands.running.take() {
            let output = result.unwrap_or_else(|error| format!("Failed to run command: {error}"));
            self.set_agent_command_notice(Some(format!("Finished {name}")));
            self.agents_overview.commands.result = Some((name, output));
            if self.chat_widget.active_view_is(AGENTS_OVERVIEW_VIEW_ID) && self.overlay.is_none() {
                self.show_agent_command_output(tui);
            }
        }
    }

    pub(super) fn show_agent_command_output(&mut self, tui: &mut tui::Tui) {
        if let Some((name, output)) = &self.agents_overview.commands.result {
            let _ = tui.enter_alt_screen();
            self.overlay = Some(Overlay::new_static_with_lines(
                output
                    .lines()
                    .map(|line| Line::from(line.to_string()))
                    .collect(),
                name.clone(),
                self.keymap.pager.clone(),
            ));
            self.set_agent_command_notice(None);
            tui.frame_requester().schedule_frame();
        }
    }

    fn set_agent_command_notice(&mut self, notice: Option<String>) {
        self.agents_overview
            .view_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .command_notice = notice;
    }
}
