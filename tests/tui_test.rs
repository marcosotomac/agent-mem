#![cfg(feature = "tui")]

use agent_mem::store::Store;
use agent_mem::tui::{ActiveTab, App, InputMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fs;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

static TUI_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_tui_app_state_and_navigation() {
    let _lock = TUI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_tui_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join(".agent-mem").join("mem.db");

    // Pre-populate DB with 3 rules and 2 sessions
    {
        let mut store = Store::open(&db_path, true).unwrap();
        store.set("arch/jwt", "JWT with RS256").unwrap();
        store.set("arch/db", "Postgres connection pooling").unwrap();
        store.set("arch/cache", "Redis cache layer").unwrap();
        store.session_add("Session 1 checkpoint").unwrap();
        store.session_add("Session 2 checkpoint").unwrap();
    }

    let mut app = App::new(temp_dir.clone()).expect("app should initialize");
    assert_eq!(app.rules.len(), 3);
    assert_eq!(app.filtered_indices.len(), 3);
    assert_eq!(app.sessions.len(), 2);
    assert_eq!(app.active_tab, ActiveTab::Rules);
    assert_eq!(app.input_mode, InputMode::Normal);

    // Tab switching (Rules -> Sessions -> Projects -> Doctor -> Help -> Rules)
    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Sessions);

    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Projects);

    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Doctor);

    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Help);

    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Rules);

    // Direct jump
    app.handle_key(key(KeyCode::Char('2'))).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Sessions);
    app.handle_key(key(KeyCode::Char('3'))).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Projects);
    app.handle_key(key(KeyCode::Char('4'))).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Doctor);
    app.handle_key(key(KeyCode::Char('?'))).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Help);
    app.handle_key(key(KeyCode::Char('1'))).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Rules);

    // Item navigation
    assert_eq!(app.selected_rule_idx, 0);
    app.handle_key(key(KeyCode::Char('j'))).unwrap();
    assert_eq!(app.selected_rule_idx, 1);
    app.handle_key(key(KeyCode::Char('k'))).unwrap();
    assert_eq!(app.selected_rule_idx, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_tui_filtering_and_search() {
    let _lock = TUI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_tui_search_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join(".agent-mem").join("mem.db");

    {
        let mut store = Store::open(&db_path, true).unwrap();
        store
            .set("frontend/react", "Use functional components")
            .unwrap();
        store
            .set("backend/rust", "Axum router and tower middleware")
            .unwrap();
        store.set("db/sqlite", "SQLite WAL mode clustered").unwrap();
    }

    let mut app = App::new(temp_dir.clone()).unwrap();
    assert_eq!(app.filtered_indices.len(), 3);

    // Enter filter mode with '/'
    app.handle_key(key(KeyCode::Char('/'))).unwrap();
    assert_eq!(app.input_mode, InputMode::Filter);

    // Type query "rust"
    for c in "rust".chars() {
        app.handle_key(key(KeyCode::Char(c))).unwrap();
    }
    assert_eq!(app.filter_query, "rust");
    assert_eq!(app.filtered_indices.len(), 1);
    assert_eq!(app.current_selected_rule().unwrap().key, "backend/rust");

    // Exit filter mode with Enter
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.filtered_indices.len(), 1);

    // Clear filter with Esc
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.filter_query, "");
    assert_eq!(app.filtered_indices.len(), 3);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_tui_archive_and_delete_actions() {
    let _lock = TUI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_tui_actions_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join(".agent-mem").join("mem.db");

    {
        let mut store = Store::open(&db_path, true).unwrap();
        store.set("arch/token", "JWT token expiry 15m").unwrap();
        store.set("arch/session", "Session in redis").unwrap();
    }

    let mut app = App::new(temp_dir.clone()).unwrap();
    assert_eq!(app.rules.len(), 2);
    assert!(!app.rules[0].is_archived());

    // 1. Toggle Archive with 'a'
    app.handle_key(key(KeyCode::Char('a'))).unwrap();
    assert!(app.current_selected_rule().unwrap().is_archived());
    assert!(app.toast.is_some());

    // 2. Reactivate with 'a' again
    app.handle_key(key(KeyCode::Char('a'))).unwrap();
    assert!(!app.current_selected_rule().unwrap().is_archived());

    // 3. Delete with confirmation 'd' -> 'n' (cancel)
    app.handle_key(key(KeyCode::Char('d'))).unwrap();
    assert_eq!(app.input_mode, InputMode::ConfirmDelete);
    app.handle_key(key(KeyCode::Char('n'))).unwrap();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.rules.len(), 2); // Nothing deleted

    // 4. Delete with confirmation 'd' -> 'y' (confirm)
    app.handle_key(key(KeyCode::Char('d'))).unwrap();
    assert_eq!(app.input_mode, InputMode::ConfirmDelete);
    app.handle_key(key(KeyCode::Char('y'))).unwrap();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.rules.len(), 1); // 1 rule remaining!

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_tui_new_rule_and_edit_modals() {
    let _lock = TUI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_tui_form_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let _db_path = temp_dir.join(".agent-mem").join("mem.db");

    let mut app = App::new(temp_dir.clone()).unwrap();
    assert_eq!(app.rules.len(), 0);

    // 1. Press 'n' to open New Rule modal
    app.handle_key(key(KeyCode::Char('n'))).unwrap();
    assert!(matches!(app.input_mode, InputMode::NewRule { .. }));

    // Type Key: "sec/ssl"
    for c in "sec/ssl".chars() {
        app.handle_key(key(KeyCode::Char(c))).unwrap();
    }
    // Switch to Value field with Tab
    app.handle_key(key(KeyCode::Tab)).unwrap();
    // Type Value: "Enforce TLS 1.3"
    for c in "Enforce TLS 1.3".chars() {
        app.handle_key(key(KeyCode::Char(c))).unwrap();
    }
    // Switch to Anchor field with Tab
    app.handle_key(key(KeyCode::Tab)).unwrap();
    // Type Anchor: "src/net.rs:10"
    for c in "src/net.rs:10".chars() {
        app.handle_key(key(KeyCode::Char(c))).unwrap();
    }
    // Submit with Enter
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.rules.len(), 1);
    assert_eq!(app.rules[0].key, "sec/ssl");
    assert_eq!(app.rules[0].val, "Enforce TLS 1.3");
    assert_eq!(app.rules[0].anchor.as_deref(), Some("src/net.rs:10"));

    // 2. Press 'e' to Edit the selected rule
    app.handle_key(key(KeyCode::Char('e'))).unwrap();
    assert!(matches!(app.input_mode, InputMode::EditRule { .. }));

    // In EditRule, field 0 is Value. Append " strictly"
    for c in " strictly".chars() {
        app.handle_key(key(KeyCode::Char(c))).unwrap();
    }
    // Tab to Anchor
    app.handle_key(key(KeyCode::Tab)).unwrap();
    // Enter to save
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.rules.len(), 1);
    assert_eq!(app.rules[0].val, "Enforce TLS 1.3 strictly");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_tui_new_session_modal_and_git_sync() {
    let _lock = TUI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_tui_session_sync_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let _db_path = temp_dir.join(".agent-mem").join("mem.db");

    let mut app = App::new(temp_dir.clone()).unwrap();
    assert_eq!(app.sessions.len(), 0);

    // 1. Press 'c' to open New Session modal
    app.handle_key(key(KeyCode::Char('c'))).unwrap();
    assert!(matches!(app.input_mode, InputMode::NewSession { .. }));

    // Type summary: "feat(auth): initial commit"
    for c in "feat(auth): initial commit".chars() {
        app.handle_key(key(KeyCode::Char(c))).unwrap();
    }
    // Submit with Enter
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.sessions[0].1, "feat(auth): initial commit");

    // 2. Trigger Git Sync with 'S'
    app.handle_key(key(KeyCode::Char('S'))).unwrap();
    assert!(app.toast.is_some());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_tui_projects_navigation_and_switch() {
    let _lock = TUI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let global_dir = std::env::temp_dir().join(format!(
        "agent_mem_tui_projects_global_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&global_dir).unwrap();
    unsafe {
        std::env::set_var("AGENT_MEM_GLOBAL_DIR", &global_dir);
    }

    let dir_a = std::env::temp_dir().join(format!(
        "agent_mem_tui_proj_a_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let dir_b = std::env::temp_dir().join(format!(
        "agent_mem_tui_proj_b_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir_a).unwrap();
    fs::create_dir_all(&dir_b).unwrap();

    let db_a = dir_a.join(".agent-mem").join("mem.db");
    let db_b = dir_b.join(".agent-mem").join("mem.db");

    {
        let mut store_a = Store::open(&db_a, true).unwrap();
        store_a.set("rule/a", "Value A").unwrap();

        let mut store_b = Store::open(&db_b, true).unwrap();
        store_b.set("rule/b", "Value B").unwrap();
    }

    // Register both projects
    let mut reg = agent_mem::registry::ProjectRegistry::load().unwrap();
    reg.register(&dir_a).unwrap();
    reg.register(&dir_b).unwrap();

    // Start App in dir_a
    let mut app = App::new(dir_a.clone()).unwrap();
    assert_eq!(app.rules.len(), 1);
    assert_eq!(app.rules[0].key, "rule/a");

    // Switch to Projects tab
    app.handle_key(key(KeyCode::Char('3'))).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Projects);
    assert!(app.projects.len() >= 2);

    // Find index of dir_b and switch
    let canonical_b = dir_b.canonicalize().unwrap();
    let target_idx = app
        .projects
        .iter()
        .position(|p| p.canonical_path == canonical_b.to_string_lossy())
        .unwrap();
    app.selected_project_idx = target_idx;
    app.handle_key(key(KeyCode::Enter)).unwrap();

    // Verify project switched to dir_b
    assert_eq!(app.root, canonical_b);
    assert_eq!(app.rules.len(), 1);
    assert_eq!(app.rules[0].key, "rule/b");

    // Clean up
    unsafe {
        std::env::remove_var("AGENT_MEM_GLOBAL_DIR");
    }
    let _ = fs::remove_dir_all(&dir_a);
    let _ = fs::remove_dir_all(&dir_b);
    let _ = fs::remove_dir_all(&global_dir);
}

#[test]
fn test_tui_rules_list_scrolling() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let _lock = TUI_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_tui_scroll_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join(".agent-mem").join("mem.db");

    // Populate 35 rules
    {
        let mut store = Store::open(&db_path, true).unwrap();
        for i in 0..35 {
            store
                .set(
                    &format!("rule/{:02}", i),
                    &format!("Description for rule {:02}", i),
                )
                .unwrap();
        }
    }

    let mut app = App::new(temp_dir.clone()).unwrap();
    assert_eq!(app.rules.len(), 35);
    assert_eq!(app.filtered_indices.len(), 35);

    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();

    // Initial render: selection is at index 0, offset must be 0
    terminal
        .draw(|f| agent_mem::tui::render_ui(f, &mut app))
        .unwrap();
    assert_eq!(app.selected_rule_idx, 0);
    assert_eq!(app.rules_state.offset(), 0);

    // Navigate down 25 times
    for _ in 0..25 {
        app.handle_key(key(KeyCode::Down)).unwrap();
    }
    assert_eq!(app.selected_rule_idx, 25);

    // Draw again - viewport MUST scroll down so rule 25 is visible!
    terminal
        .draw(|f| agent_mem::tui::render_ui(f, &mut app))
        .unwrap();
    assert!(
        app.rules_state.offset() > 0,
        "List viewport offset must be > 0 when selection moves past visible height, was {}",
        app.rules_state.offset()
    );

    // Test End key jumps to the very end
    app.handle_key(key(KeyCode::End)).unwrap();
    assert_eq!(app.selected_rule_idx, 34);
    terminal
        .draw(|f| agent_mem::tui::render_ui(f, &mut app))
        .unwrap();
    assert!(
        app.rules_state.offset() >= 15,
        "Offset at end of 35 items should be >= 15, was {}",
        app.rules_state.offset()
    );

    // Test Home key jumps back to the beginning
    app.handle_key(key(KeyCode::Home)).unwrap();
    assert_eq!(app.selected_rule_idx, 0);
    terminal
        .draw(|f| agent_mem::tui::render_ui(f, &mut app))
        .unwrap();
    assert_eq!(app.rules_state.offset(), 0);

    // Test PageDown jumps by 10
    app.handle_key(key(KeyCode::PageDown)).unwrap();
    assert_eq!(app.selected_rule_idx, 10);

    // Test PageUp jumps back by 10
    app.handle_key(key(KeyCode::PageUp)).unwrap();
    assert_eq!(app.selected_rule_idx, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}
