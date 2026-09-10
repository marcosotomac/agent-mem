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
            val: "v1 v2".into()
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
