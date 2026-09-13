use agent_mem::cli::{parse_args, Command};
use agent_mem::init::find_project_root_from;
use std::fs;

#[test]
fn test_parse_args() {
    // Empty args defaults to help
    assert_eq!(parse_args(vec!["agent-mem".into()]).unwrap(), Command::Help);

    // Help
    assert_eq!(parse_args(vec!["agent-mem".into(), "--help".into()]).unwrap(), Command::Help);

    // Version
    assert_eq!(parse_args(vec!["agent-mem".into(), "-v".into()]).unwrap(), Command::Version);

    // Init
    assert_eq!(parse_args(vec!["agent-mem".into(), "init".into()]).unwrap(), Command::Init);

    // Set
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "set".into(), "k".into(), "v1".into(), "v2".into()]).unwrap(),
        Command::Set {
            key: "k".into(),
            val: "v1 v2".into(),
            anchor: None,
        }
    );

    // Set with anchor
    assert_eq!(
        parse_args(vec![
            "agent-mem".into(),
            "set".into(),
            "k".into(),
            "v1".into(),
            "--anchor".into(),
            "src/lib.rs:42".into()
        ])
        .unwrap(),
        Command::Set {
            key: "k".into(),
            val: "v1".into(),
            anchor: Some("src/lib.rs:42".into()),
        }
    );

    // Get
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "get".into(), "k".into()]).unwrap(),
        Command::Get { key: "k".into() }
    );

    // Del
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "del".into(), "k".into()]).unwrap(),
        Command::Del { key: "k".into() }
    );

    // Find
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "find".into(), "foo".into(), "bar".into()]).unwrap(),
        Command::Find { query: "foo bar".into() }
    );

    // Session add
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "session".into(), "add".into(), "checkpoint".into()]).unwrap(),
        Command::SessionAdd { summary: "checkpoint".into() }
    );

    // Session list
    assert_eq!(
        parse_args(vec!["agent-mem".into(), "session".into(), "list".into()]).unwrap(),
        Command::SessionList
    );

    // Context
    assert_eq!(parse_args(vec!["agent-mem".into(), "context".into()]).unwrap(), Command::Context);

    // Missing required args return error
    assert!(parse_args(vec!["agent-mem".into(), "get".into()]).is_err());
    assert!(parse_args(vec!["agent-mem".into(), "unknown-cmd".into()]).is_err());
}

#[test]
fn test_root_traversal() {
    let temp_dir = std::env::temp_dir().join(format!("agent_mem_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let sub_dir = temp_dir.join("a").join("b").join("c");
    fs::create_dir_all(&sub_dir).unwrap();

    // Create marker at temp_dir
    let git_marker = temp_dir.join(".git");
    fs::create_dir_all(&git_marker).unwrap();

    let found = find_project_root_from(&sub_dir);
    assert_eq!(found, temp_dir);

    // Cleanup
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_read_does_not_mutate_uninitialized_directory() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_uninit_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Get on uninitialized directory must fail without creating .agent-mem
    let cmd = Command::Get {
        key: "some_key".into(),
    };
    let res = agent_mem::cli::execute_command(cmd, &temp_dir);
    assert!(matches!(res, Err(agent_mem::error::Error::NotInitialized)));
    assert!(!temp_dir.join(".agent-mem").exists());

    // 2. Dump on uninitialized directory must also fail without creating .agent-mem
    let dump_cmd = Command::Dump;
    let dump_res = agent_mem::cli::execute_command(dump_cmd, &temp_dir);
    assert!(matches!(dump_res, Err(agent_mem::error::Error::NotInitialized)));
    assert!(!temp_dir.join(".agent-mem").exists());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_get_nonexistent_returns_not_found() {
    let temp_dir = std::env::temp_dir().join(format!(
        "agent_mem_cli_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    // Initialize first so store exists
    agent_mem::init::init_project(&temp_dir).unwrap();

    let cmd = Command::Get {
        key: "nonexistent_rule".into(),
    };
    let res = agent_mem::cli::execute_command(cmd, &temp_dir);

    match res {
        Err(agent_mem::error::Error::NotFound(k)) => assert_eq!(k, "nonexistent_rule"),
        other => panic!("expected NotFound error, got {:?}", other),
    }

    let _ = fs::remove_dir_all(&temp_dir);
}
