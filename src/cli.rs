use crate::error::{Error, Result};
use crate::init::{find_project_root_from, init_project};
use crate::output::*;
use crate::store::Store;
use std::env;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Init {
        path: Option<String>,
    },
    Get {
        key: String,
    },
    Set {
        key: String,
        val: String,
        anchor: Option<String>,
    },
    Del {
        key: String,
    },
    Archive {
        key: String,
        reason: Option<String>,
    },
    Unarchive {
        key: String,
    },
    Find {
        query: String,
    },
    Dump,
    Context,
    SessionAdd {
        summary: String,
    },
    SessionList,
    Sync {
        file: Option<String>,
        export: bool,
    },
    Mcp,
    McpInstall {
        client: Option<String>,
    },
    Doctor,
    Tui,
    Help,
    Version,
}

impl Command {
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Command::Set { .. }
                | Command::Del { .. }
                | Command::Archive { .. }
                | Command::Unarchive { .. }
                | Command::SessionAdd { .. }
                | Command::Sync { export: false, .. }
        )
    }
}

pub fn parse_args<I>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    if args.len() < 2 {
        return Ok(Command::Help);
    }

    let cmd_str = args[1].as_str();
    match cmd_str {
        "init" => Ok(Command::Init {
            path: args.get(2).cloned(),
        }),
        "get" => {
            if args.len() < 3 {
                return Err(Error::Usage("Usage: agent-mem get <key>".to_string()));
            }
            Ok(Command::Get {
                key: args[2].clone(),
            })
        }
        "set" => {
            if args.len() < 4 {
                return Err(Error::Usage(
                    "Usage: agent-mem set <key> <value> [--anchor <path:line>]".to_string(),
                ));
            }
            let key = args[2].clone();
            let mut val_parts = Vec::new();
            let mut anchor = None;
            let mut i = 3;
            while i < args.len() {
                if (args[i] == "--anchor" || args[i] == "-a") && i + 1 < args.len() {
                    anchor = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    val_parts.push(args[i].clone());
                    i += 1;
                }
            }
            let val = val_parts.join(" ");
            Ok(Command::Set { key, val, anchor })
        }
        "del" | "rm" => {
            if args.len() < 3 {
                return Err(Error::Usage("Usage: agent-mem del <key>".to_string()));
            }
            Ok(Command::Del {
                key: args[2].clone(),
            })
        }
        "archive" => {
            if args.len() < 3 {
                return Err(Error::Usage(
                    "Usage: agent-mem archive <key> [--reason <reason>]".to_string(),
                ));
            }
            let key = args[2].clone();
            let mut reason = None;
            let mut i = 3;
            while i < args.len() {
                if (args[i] == "--reason" || args[i] == "-r") && i + 1 < args.len() {
                    reason = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    reason = Some(args[i..].join(" "));
                    break;
                }
            }
            Ok(Command::Archive { key, reason })
        }
        "unarchive" => {
            if args.len() < 3 {
                return Err(Error::Usage("Usage: agent-mem unarchive <key>".to_string()));
            }
            Ok(Command::Unarchive {
                key: args[2].clone(),
            })
        }
        "dump" | "list" | "ls" => Ok(Command::Dump),
        "find" | "search" => {
            if args.len() < 3 {
                return Err(Error::Usage("Usage: agent-mem find <query>".to_string()));
            }
            let query = args[3..].iter().fold(args[2].clone(), |mut acc, s| {
                acc.push(' ');
                acc.push_str(s);
                acc
            });
            Ok(Command::Find { query })
        }
        "session" => {
            if args.len() < 3 {
                return Err(Error::Usage(
                    "Usage: agent-mem session [add <msg> | list]".to_string(),
                ));
            }
            match args[2].as_str() {
                "add" => {
                    if args.len() < 4 {
                        return Err(Error::Usage(
                            "Usage: agent-mem session add <summary>".to_string(),
                        ));
                    }
                    let summary = args[3..].join(" ");
                    Ok(Command::SessionAdd { summary })
                }
                "list" | "ls" => Ok(Command::SessionList),
                other => Err(Error::Usage(format!(
                    "Unknown session subcommand '{}'. Usage: agent-mem session [add|list]",
                    other
                ))),
            }
        }
        "context" => Ok(Command::Context),
        "sync" => {
            let mut file = None;
            let mut export = false;
            for arg in &args[2..] {
                if arg == "--export" || arg == "-e" {
                    export = true;
                } else if !arg.starts_with('-') && file.is_none() {
                    file = Some(arg.clone());
                }
            }
            Ok(Command::Sync { file, export })
        }
        "mcp" => {
            if args.len() >= 3 && args[2] == "install" {
                let client = if args.len() >= 4 {
                    Some(args[3].clone())
                } else {
                    None
                };
                Ok(Command::McpInstall { client })
            } else {
                Ok(Command::Mcp)
            }
        }
        "--mcp" => Ok(Command::Mcp),
        "tui" | "ui" => Ok(Command::Tui),
        "doctor" => Ok(Command::Doctor),
        "help" | "--help" | "-h" => Ok(Command::Help),
        "version" | "--version" | "-v" => Ok(Command::Version),
        unknown => Err(Error::Usage(format!(
            "Unknown command '{}'. Run 'agent-mem --help' for usage.",
            unknown
        ))),
    }
}

pub fn execute_command(cmd: Command, root: &Path) -> Result<()> {
    match cmd {
        Command::Help => {
            print_help();
            Ok(())
        }
        Command::Version => {
            println!("agent-mem {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Init { .. } => {
            let report = init_project(root)?;
            print_init(&report);
            Ok(())
        }
        Command::Mcp => {
            let server = crate::mcp::McpServer::new();
            server.run_stdio()
        }
        Command::McpInstall { client } => {
            use std::io::IsTerminal;
            let results = crate::installer::install_all_or_target(client.as_deref())?;
            if results.is_empty() {
                println!(
                    "No supported clients detected. Run 'agent-mem mcp install [windsurf|cursor|vscode|zed|claude|opencode]'."
                );
            } else {
                let is_tty = std::io::stdout().is_terminal();
                for r in results {
                    let status = if r.already_configured {
                        "updated"
                    } else {
                        "configured"
                    };
                    if is_tty {
                        println!(
                            "  \x1b[38;5;150m✓\x1b[0m  \x1b[38;5;245m{}\x1b[0m  \x1b[1;37m{}\x1b[0m  \x1b[38;5;240m·\x1b[0m  \x1b[38;5;245m{}\x1b[0m",
                            status,
                            r.client.display_name(),
                            r.path.display()
                        );
                    } else {
                        println!(
                            "{} {} in {}",
                            status,
                            r.client.display_name(),
                            r.path.display()
                        );
                    }
                }
            }
            Ok(())
        }
        Command::Tui => {
            #[cfg(feature = "tui")]
            {
                crate::tui::run(root)
            }
            #[cfg(not(feature = "tui"))]
            {
                Err(Error::Usage(
                    "TUI support is not compiled into this build of agent-mem. Reinstall with '--features tui'.".to_string(),
                ))
            }
        }
        Command::Doctor => {
            let db_path = root.join(".agent-mem").join("mem.db");
            let store_stats = if db_path.exists() {
                Store::open(&db_path, false)
                    .ok()
                    .and_then(|s| s.stats(&db_path).ok())
            } else {
                None
            };
            let git_stats = crate::init::inspect_git_health(root);
            let clients = crate::installer::check_all_clients();
            print_doctor(store_stats.as_ref(), &git_stats, &clients);
            Ok(())
        }
        _ => {
            let db_path = root.join(".agent-mem").join("mem.db");
            let mut store = Store::open(&db_path, cmd.is_write())?;

            match cmd {
                Command::Get { key } => {
                    if let Some(record) = store.get_entry(&key)? {
                        print_get_record(&record);
                    } else {
                        return Err(crate::error::Error::NotFound(key));
                    }
                }
                Command::Set { key, val, anchor } => {
                    store.set_with_anchor(&key, &val, anchor.as_deref())?;
                    let rules_file = root.join(".agent-rules");
                    if rules_file.exists() {
                        let _ = store.export_to_file(&rules_file);
                    }
                    print_set(&key, anchor.as_deref());
                }
                Command::Del { key } => {
                    let deleted = store.del(&key)?;
                    let rules_file = root.join(".agent-rules");
                    if rules_file.exists() {
                        let _ = store.export_to_file(&rules_file);
                    }
                    print_del(&key, deleted);
                }
                Command::Archive { key, reason } => {
                    let archived = store.archive(&key, reason.as_deref())?;
                    if !archived {
                        return Err(crate::error::Error::NotFound(key));
                    }
                    let rules_file = root.join(".agent-rules");
                    if rules_file.exists() {
                        let _ = store.export_to_file(&rules_file);
                    }
                    print_archive(&key, reason.as_deref());
                }
                Command::Unarchive { key } => {
                    let unarchived = store.unarchive(&key)?;
                    if !unarchived {
                        return Err(crate::error::Error::NotFound(key));
                    }
                    let rules_file = root.join(".agent-rules");
                    if rules_file.exists() {
                        let _ = store.export_to_file(&rules_file);
                    }
                    print_unarchive(&key);
                }
                Command::Dump => {
                    let entries = store.dump()?;
                    print_dump(&entries);
                }
                Command::Find { query } => {
                    let entries = store.find(&query)?;
                    print_find(&query, &entries);
                }
                Command::SessionAdd { summary } => {
                    let id = store.session_add(&summary)?;
                    print_session_add(id);
                }
                Command::SessionList => {
                    let list = store.session_list(5)?;
                    print_session_list(&list);
                }
                Command::Context => {
                    let (rules, sessions) = store.context()?;
                    print_context(&rules, &sessions);
                }
                Command::Sync { file, export } => {
                    let target_file = file.as_deref().unwrap_or(".agent-rules");
                    let file_path = if Path::new(target_file).is_absolute() {
                        Path::new(target_file).to_path_buf()
                    } else {
                        root.join(target_file)
                    };
                    let report = if export {
                        store.sync_export(&file_path)?
                    } else {
                        store.sync_with_file(&file_path)?
                    };
                    print_sync(&report);
                }
                Command::Init { .. }
                | Command::Help
                | Command::Version
                | Command::Doctor
                | Command::Tui
                | Command::Mcp
                | Command::McpInstall { .. } => unreachable!(),
            }
            Ok(())
        }
    }
}

pub fn run() -> Result<()> {
    let cmd = parse_args(env::args())?;
    let cwd = env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let root = match &cmd {
        Command::Init { path: Some(p) } => {
            let target = std::path::PathBuf::from(p);
            if target.is_absolute() {
                target
            } else {
                cwd.join(target)
            }
        }
        Command::Init { path: None } => cwd,
        _ => find_project_root_from(&cwd),
    };
    execute_command(cmd, &root)
}
