use agent_mem::store::Store;
use rusqlite::Connection;
use std::{fs, process::Command};
mod support;
use support::TempDir;

fn doctor_fix(dir: &TempDir) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agent-mem"))
        .args(["doctor", "--fix"])
        .current_dir(&dir.0)
        .env("AGENT_MEM_GLOBAL_DIR", dir.0.join("isolated-global"))
        .output()
        .unwrap()
}

#[test]
fn doctor_rejects_future_schema_without_losing_history_or_sessions() {
    let dir = TempDir::new();
    {
        let mut store = Store::open(&dir.db(), true).unwrap();
        store.set("safety/check", "Original guidance").unwrap();
        store.set("safety/check", "Updated guidance").unwrap();
        store.session_add("Keep this checkpoint").unwrap();
        store.export_to_file(&dir.0.join(".agent-rules")).unwrap();
    }
    let conn = Connection::open(dir.db()).unwrap();
    conn.execute_batch("PRAGMA user_version = 8;").unwrap();
    drop(conn);
    let rules_before = fs::read(dir.0.join(".agent-rules")).unwrap();
    let result = doctor_fix(&dir);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("newer than supported"));
    let conn = Connection::open(dir.db()).unwrap();
    for (query, expected) in [
        ("PRAGMA user_version", 8),
        ("SELECT COUNT(*) FROM sessions", 1),
        ("SELECT COUNT(*) FROM memory_revisions", 1),
        ("SELECT COUNT(*) FROM memories", 1),
    ] {
        assert_eq!(
            conn.query_row(query, [], |r| r.get::<_, i64>(0)).unwrap(),
            expected
        );
    }
    assert_eq!(fs::read(dir.0.join(".agent-rules")).unwrap(), rules_before);
    assert!(!fs::read_dir(dir.0.join(".agent-mem")).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("mem.db.backup-")
    }));
}

#[test]
fn doctor_backs_up_corruption_before_rebuilding_from_modular_rules() {
    let dir = TempDir::new();
    fs::create_dir_all(dir.0.join(".agent-mem")).unwrap();
    let original = b"corrupted sqlite database junk";
    fs::write(dir.db(), original).unwrap();
    fs::create_dir(dir.0.join(".agent-rules.d")).unwrap();
    fs::write(
        dir.0.join(".agent-rules.d/safety.rules"),
        "safety/check = Preserved guidance\n",
    )
    .unwrap();
    let result = doctor_fix(&dir);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let backup = fs::read_dir(dir.0.join(".agent-mem"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("mem.db.backup-")
        })
        .expect("repair must keep a backup");
    assert_eq!(fs::read(backup.join("mem.db")).unwrap(), original);
    assert_eq!(
        Store::open(&dir.db(), false)
            .unwrap()
            .get("safety/check")
            .unwrap()
            .as_deref(),
        Some("Preserved guidance")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
