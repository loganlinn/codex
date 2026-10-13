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
    app.run_agent_command("inspect".to_string(), thread);
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
    app.run_agent_command("inspect".to_string(), thread);
    // The only event is the visible error, never a process completion.
    assert!(matches!(rx.try_recv(), Ok(AppEvent::InsertHistoryCell(_))));
    assert!(rx.try_recv().is_err());
    assert!(
        app.agents_overview
            .view_state
            .lock()
            .unwrap()
            .command_notice
            .is_none()
    );
}
