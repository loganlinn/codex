use super::*;
use crate::bottom_pane::BottomPaneView;
use codex_config::types::Tui;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn custom_command_menu_shows_configured_names_and_descriptions() {
    let mut app = make_test_app().await;
    app.local_settings.tui = toml::from_str::<Tui>(
        r#"
        [agent_commands.open_editor]
        description = "Open this checkout in the editor"
        interpreter = "code"
        args = ["{{cwd}}"]
        [agent_commands.inspect]
        interpreter = "git"
        args = ["status", "--short"]
    "#,
    )
    .unwrap();
    let thread = overview_thread(
        ThreadId::from_u128(42),
        None,
        "Selected task",
        ThreadStatus::Idle,
    );
    app.open_agent_commands(Some(thread));
    let screen = render_bottom_popup(&app.chat_widget, 100);
    assert!(screen.contains("open_editor"));
    assert!(screen.contains("inspect"));
    assert!(screen.contains("Selected task"));
    insta::assert_snapshot!("custom_command_menu", screen);
}

#[tokio::test]
async fn command_menu_uses_selected_task_and_does_not_intercept_metadata() {
    let mut app = make_test_app().await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.app_event_tx = AppEventSender::new(tx);
    let id = ThreadId::new();
    let thread = overview_thread(id, None, "Selected task", ThreadStatus::Idle);
    let mut view = app.agents_overview_view(vec![thread], Some(id));
    view.handle_key_event(KeyCode::Char('c').into());
    let AppEvent::OpenAgentCommands { thread } = rx.try_recv().unwrap() else {
        panic!("expected command menu");
    };
    assert_eq!(thread.unwrap().id, id.to_string());
    view.handle_key_event(KeyCode::Char('/').into());
    view.handle_key_event(KeyCode::Char('c').into());
    assert!(rx.try_recv().is_err());
    view.handle_key_event(KeyCode::Esc.into());
    view.handle_key_event(KeyCode::Char('r').into());
    view.handle_key_event(KeyCode::Char('c').into());
    assert!(rx.try_recv().is_err());
    view.handle_key_event(KeyCode::Esc.into());
    let mut empty = app.agents_overview_view(Vec::new(), None);
    empty.handle_key_event(KeyCode::Char('c').into());
    assert!(matches!(
        rx.try_recv(),
        Ok(AppEvent::OpenAgentCommands { thread: None })
    ));
    app.agents_overview
        .view_state
        .lock()
        .unwrap()
        .connection_notice = Some("Disconnected");
    empty.handle_key_event(KeyCode::Char('c').into());
    assert!(matches!(
        rx.try_recv(),
        Ok(AppEvent::OpenAgentCommands { thread: None })
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn configured_command_preserves_arguments_environment_and_selected_cwd() {
    let mut app = make_test_app().await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.app_event_tx = AppEventSender::new(tx);
    app.local_settings.tui = toml::from_str::<Tui>(r#"
        [agent_commands.inspect]
        interpreter = "/bin/sh"
        args = ["-c", 'printf "%s\\n" "$1" "$SELECTED_ID" "$PWD"; printf failure >&2; exit 7', "inspect", "{{title}}"]
        [agent_commands.inspect.environment]
        SELECTED_ID = "{{ thread_id }}"
    "#).unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(cwd.path()).unwrap();
    let id = ThreadId::new();
    let title = "A title with spaces; $(echo injected) {{cwd}}";
    let mut thread = overview_thread(id, None, title, ThreadStatus::Idle);
    thread.cwd = AbsolutePathBuf::from_absolute_path(&cwd).unwrap();
    let server = crate::start_embedded_app_server_for_picker(&app.config)
        .await
        .unwrap();
    app.run_agent_command(&server, "inspect".to_string(), thread);
    let event = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let AppEvent::AgentCommandFinished { request_id, result } = event else {
        panic!("expected command completion");
    };
    let output = result.unwrap();
    assert!(output.contains("exit status: 7"), "{output}");
    assert!(output.contains(title), "{output}");
    assert!(output.contains(&id.to_string()), "{output}");
    assert!(output.contains(&cwd.display().to_string()), "{output}");
    assert!(output.contains("stderr:\nfailure"), "{output}");
    let mut tui = crate::tui::test_support::make_test_tui().unwrap();
    app.finish_agent_command(&mut tui, request_id, Ok(output));
    app.show_agent_command_output(&mut tui);
    assert!(app.overlay.is_some());
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_template_does_not_start_a_process() {
    let (mut app, mut rx, _op_rx) = make_test_app_with_channels().await;
    app.local_settings.tui = toml::from_str::<Tui>(
        r#"
        [agent_commands.inspect]
        interpreter = "{{missing}}"
    "#,
    )
    .unwrap();
    let thread = overview_thread(ThreadId::new(), None, "Task", ThreadStatus::Idle);
    let server = crate::start_embedded_app_server_for_picker(&app.config)
        .await
        .unwrap();
    app.run_agent_command(&server, "inspect".to_string(), thread);
    let event = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let AppEvent::AgentCommandFinished { result, .. } = event else {
        panic!("expected command failure");
    };
    assert!(result.unwrap_err().contains("unknown template variable"));
    assert!(rx.try_recv().is_err());
    server.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn context_file_uses_selected_session_metadata_and_is_removed_after_exit() {
    let (mut app, mut rx, _op_rx) = make_test_app_with_channels().await;
    app.local_settings.tui = toml::from_str::<Tui>(r#"
        [agent_commands.inspect]
        interpreter = "/bin/sh"
        args = ["-c", 'test "$1" = "$CODEX_AGENT_CONTEXT_FILE" && test "$2" = "$PARENT_ID" && test -z "$3" && printf "%s\\n" "$1" && cat "$1"', "inspect", "{{context_file}}", "{{parent_thread_id}}", "{{model}}"]
        [agent_commands.inspect.environment]
        PARENT_ID = "{{session_id}}"
    "#).unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let id = ThreadId::new();
    let parent = ThreadId::new();
    let mut thread = overview_thread(
        id,
        Some(parent),
        "Selected child",
        ThreadStatus::Active {
            active_flags: vec![codex_app_server_protocol::ThreadActiveFlag::WaitingOnUserInput],
        },
    );
    thread.cwd = AbsolutePathBuf::from_absolute_path(cwd.path()).unwrap();
    thread.git_info = Some(codex_app_server_protocol::GitInfo {
        sha: Some("selected-sha".into()),
        branch: Some("selected-branch".into()),
        origin_url: None,
    });
    app.agents_overview
        .last_messages
        .insert(id, "Selected preview".into());
    app.agents_overview
        .last_messages
        .insert(parent, "Wrong preview".into());
    app.agents_overview.usage.entry(id).or_default().tokens =
        Some(codex_app_server_protocol::TokenUsageBreakdown {
            total_tokens: 123,
            input_tokens: 100,
            cached_input_tokens: 50,
            cache_write_input_tokens: 0,
            output_tokens: 23,
            reasoning_output_tokens: 10,
        });
    let server = crate::start_embedded_app_server_for_picker(&app.config)
        .await
        .unwrap();
    app.run_agent_command(&server, "inspect".to_string(), thread);
    let event = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let AppEvent::AgentCommandFinished { result, .. } = event else {
        panic!("expected command completion");
    };
    let output = result.unwrap();
    let stdout = output.split_once("stdout:\n").unwrap().1;
    let (path, document) = stdout.split_once('\n').unwrap();
    let document: serde_json::Value = serde_json::from_str(document).unwrap();
    assert_eq!(document["schema_version"], 1);
    assert_eq!(document["session"]["thread_id"], id.to_string());
    assert_eq!(document["session"]["parent_thread_id"], parent.to_string());
    assert_eq!(document["session"]["status"], "active");
    assert_eq!(
        document["active_flags"],
        serde_json::json!(["waiting_on_user_input"])
    );
    assert_eq!(document["session"]["git_branch"], "selected-branch");
    assert_eq!(document["session"]["git_sha"], "selected-sha");
    assert_eq!(document["last_message_preview"], "Selected preview");
    assert_eq!(document["host"]["tui_pid"], std::process::id());
    assert!(document["session"]["model"].is_null());
    assert_eq!(document["token_usage"]["total_tokens"], 123);
    assert!(document["messages"].is_null());
    assert!(!std::path::Path::new(path).exists());
    server.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn requested_messages_are_fetched_from_legacy_and_paginated_sessions() {
    for paginated in [false, true] {
        let (mut app, mut rx, _op_rx) = make_test_app_with_channels().await;
        app.local_settings.tui = toml::from_str::<Tui>(
            r#"
            [agent_commands.inspect]
            interpreter = "/bin/cat"
            args = ["{{context_file}}"]
            message_turns = 2
        "#,
        )
        .unwrap();
        let message = "A full message; $(literal) {{cwd}}\n".repeat(30);
        let id = if paginated {
            create_paginated_message_session(&app.config, &message).await
        } else {
            app_test_support::create_fake_rollout(
                &app.config.codex_home,
                "2025-01-05T12-00-00",
                "2025-01-05T12:00:00Z",
                &message,
                Some(&app.config.model_provider_id),
                None,
            )
            .unwrap()
        };
        let mut thread = overview_thread(
            ThreadId::from_string(&id).unwrap(),
            None,
            "Task",
            ThreadStatus::NotLoaded,
        );
        let directory = tempfile::tempdir().unwrap();
        thread.cwd = AbsolutePathBuf::from_absolute_path(directory.path()).unwrap();
        let server = crate::start_embedded_app_server_for_picker(&app.config)
            .await
            .unwrap();
        app.run_agent_command(&server, "inspect".to_string(), thread);
        let event = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let AppEvent::AgentCommandFinished { result, .. } = event else {
            panic!("expected command completion");
        };
        let output = result.unwrap();
        let document: serde_json::Value =
            serde_json::from_str(output.split_once("stdout:\n").unwrap().1).unwrap();
        assert_eq!(
            document["messages"][0]["role"], "user",
            "paginated={paginated}; {document}"
        );
        assert_eq!(document["messages"][0]["text"], message);
        if paginated {
            assert_eq!(document["messages"][1]["text"], "Saved assistant message");
        }
        assert_eq!(document["messages_has_more"], false);
        assert_eq!(document["message_turns"], 1);
        assert!(document["messages_fetched_at"].is_number());
        server.shutdown().await.unwrap();
    }
}

#[cfg(unix)]
async fn create_paginated_message_session(
    config: &crate::legacy_core::config::Config,
    message: &str,
) -> String {
    use codex_thread_store::ThreadStore;

    // Use the store writer so both the rollout and its indexed projection exist.
    let thread_id = ThreadId::new();
    let sqlite = config.sqlite_config().clone();
    let state = codex_state::StateRuntime::init(sqlite.clone(), config.model_provider_id.clone())
        .await
        .unwrap();
    let store = codex_thread_store::LocalThreadStore::new(
        codex_thread_store::LocalThreadStoreConfig {
            codex_home: config.codex_home.to_path_buf(),
            sqlite,
            default_model_provider_id: config.model_provider_id.clone(),
        },
        Some(state),
    );
    store
        .create_thread(codex_thread_store::CreateThreadParams {
            creator_user_id: None,
            creator_account_id: None,
            session_id: thread_id.into(),
            thread_id,
            extra_config: None,
            forked_from_id: None,
            parent_thread_id: None,
            source: codex_protocol::protocol::SessionSource::Cli,
            thread_source: None,
            originator: "test".into(),
            base_instructions: Default::default(),
            dynamic_tools: Vec::new(),
            selected_capability_roots: Vec::new(),
            multi_agent_version: None,
            history_mode: codex_protocol::protocol::ThreadHistoryMode::Paginated,
            history_base: None,
            subagent_history_start_ordinal: None,
            initial_window_id: Uuid::new_v4().to_string(),
            runtime_workspace_roots: None,
            metadata: codex_thread_store::ThreadPersistenceMetadata {
                cwd: Some(config.codex_home.to_path_buf()),
                model_provider: config.model_provider_id.clone(),
                memory_mode: codex_protocol::protocol::ThreadMemoryMode::Enabled,
            },
        })
        .await
        .unwrap();
    store
        .persist_thread(thread_id, codex_thread_store::PersistContext::Standard)
        .await
        .unwrap();
    let events = [
        serde_json::json!({"type": "task_started", "turn_id": "turn-1"}),
        serde_json::json!({
            "type": "item_completed", "thread_id": thread_id, "turn_id": "turn-1",
            "item": {"type": "UserMessage", "id": "user-1", "content": [{"type": "text", "text": message}]}
        }),
        serde_json::json!({
            "type": "item_completed", "thread_id": thread_id, "turn_id": "turn-1",
            "item": {"type": "AgentMessage", "id": "assistant-1", "content": [{"type": "Text", "text": "Saved assistant message"}]}
        }),
        serde_json::json!({"type": "task_complete", "turn_id": "turn-1"}),
    ];
    store
        .append_items(codex_thread_store::AppendThreadItemsParams {
            thread_id,
            items: events
                .into_iter()
                .map(|event| {
                    codex_rollout::RolloutItem::EventMsg(serde_json::from_value(event).unwrap())
                })
                .collect(),
        })
        .await
        .unwrap();
    store.shutdown_thread(thread_id).await.unwrap();
    thread_id.to_string()
}

#[cfg(unix)]
#[tokio::test]
async fn missing_requested_history_prevents_execution() {
    let (mut app, mut rx, _op_rx) = make_test_app_with_channels().await;
    let directory = tempfile::tempdir().unwrap();
    app.local_settings.tui = toml::from_str::<Tui>(
        r#"
        [agent_commands.inspect]
        interpreter = "/usr/bin/touch"
        args = ["should-not-exist"]
        message_turns = 1
    "#,
    )
    .unwrap();
    let mut thread = overview_thread(ThreadId::new(), None, "Task", ThreadStatus::NotLoaded);
    thread.cwd = AbsolutePathBuf::from_absolute_path(directory.path()).unwrap();
    let server = crate::start_embedded_app_server_for_picker(&app.config)
        .await
        .unwrap();
    app.run_agent_command(&server, "inspect".to_string(), thread);
    let event = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let AppEvent::AgentCommandFinished { result, .. } = event else {
        panic!("expected command failure");
    };
    assert!(
        result
            .unwrap_err()
            .contains("cannot fetch custom command messages")
    );
    assert!(!directory.path().join("should-not-exist").exists());
    server.shutdown().await.unwrap();
}
