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

    // Tab switching
    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Sessions);

    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Help);

    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Rules);

    // Direct jump
    app.handle_key(key(KeyCode::Char('2'))).unwrap();
    assert_eq!(app.active_tab, ActiveTab::Sessions);
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
        store.set("frontend/react", "Use functional components").unwrap();
        store.set("backend/rust", "Axum router and tower middleware").unwrap();
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
