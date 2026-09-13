#![cfg(feature = "tui")]

use agent_mem::store::Store;
use agent_mem::tui::{ActiveTab, App, InputMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fs;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn test_tui_app_state_and_navigation() {
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

    // Tab switching (Rules -> Sessions -> Doctor -> Help -> Rules)
    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Sessions);

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
    assert_eq!(app.active_tab, ActiveTab::Doctor);
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
