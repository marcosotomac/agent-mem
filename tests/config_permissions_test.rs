#![cfg(unix)]
use agent_mem::installer::{TargetClient, install_to_path, uninstall_from_path};
use std::fs;
mod support;
use support::TempDir;

#[cfg(unix)]
#[test]
fn installer_preserves_config_and_backup_permissions_and_uses_private_defaults() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new();
    for (client, filename, original) in [
        (
            TargetClient::Cursor,
            "mcp.json",
            "{\"mcpServers\":{\"other\":{\"command\":\"other\",\"env\":{\"TOKEN\":\"keep-private\"}}}}",
        ),
        (
            TargetClient::Codex,
            "config.toml",
            "[mcp_servers.other]\ncommand = \"other\"\n",
        ),
    ] {
        for mode in [0o600, 0o640] {
            let path = dir.0.join(format!("{mode}-{filename}"));
            fs::write(&path, original).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            let backup = path.with_file_name(format!("{mode}-{filename}.agent-mem.bak"));
            fs::write(&backup, "old backup").unwrap();
            fs::set_permissions(&backup, fs::Permissions::from_mode(0o644)).unwrap();
            install_to_path(&path, client).unwrap();
            assert_eq!(fs::read_to_string(&backup).unwrap(), original);
            for target in [&path, &backup] {
                assert_eq!(
                    fs::metadata(target).unwrap().permissions().mode() & 0o777,
                    mode
                );
            }
            uninstall_from_path(&path, client).unwrap();
            for target in [&path, &backup] {
                assert_eq!(
                    fs::metadata(target).unwrap().permissions().mode() & 0o777,
                    mode
                );
            }
        }
        let path = dir.0.join(format!("new-{filename}"));
        install_to_path(&path, client).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
