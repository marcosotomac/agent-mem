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
        kind: Option<String>,
    },
    Relate {
        source: String,
        rel_type: String,
        target: String,
    },
    Unrelate {
        source: String,
        rel_type: String,
        target: String,
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
    Trust {
        key: String,
        reviewed: bool,
    },
    Metadata {
        key: String,
    },
    History {
        key: String,
    },
    Find {
        query: String,
    },
    Dump,
    Context {
        anchor: Option<String>,
        topic: Option<String>,
        limit: usize,
        diff: bool,
        files: Vec<String>,
    },
    SessionAdd {
        summary: String,
    },
    SessionList,
    Sync {
        file: Option<String>,
        export: bool,
    },
    SyncAcceptConflicts {
        file: Option<String>,
    },
    SemanticBuild,
    SemanticStatus,
    SemanticClear,
    Mcp {
        read_only: bool,
    },
    McpInstall {
        client: Option<String>,
    },
    McpUninstall {
        client: Option<String>,
    },
    Doctor,
    Projects {
        prune: bool,
    },
    Clean {
        dry_run: bool,
    },
    HookPostCommit {
        dry_run: bool,
    },
    Tui,
    Help,
    Version,
}

impl Command {
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Command::Set { .. }
                | Command::Relate { .. }
                | Command::Unrelate { .. }
                | Command::Del { .. }
                | Command::Archive { .. }
                | Command::Unarchive { .. }
                | Command::Trust { .. }
                | Command::Clean { dry_run: false }
                | Command::SessionAdd { .. }
                | Command::Sync { export: false, .. }
                | Command::SyncAcceptConflicts { .. }
                | Command::SemanticBuild
                | Command::SemanticClear
                | Command::HookPostCommit { dry_run: false }
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
                    "Usage: agent-mem set <key> <value> [--anchor <path:line>] [--kind <kind>]"
                        .to_string(),
                ));
            }
            let key = args[2].clone();
            let mut val_parts = Vec::new();
            let mut anchor = None;
            let mut kind = None;
            let mut i = 3;
            while i < args.len() {
                if (args[i] == "--anchor" || args[i] == "-a") && i + 1 < args.len() {
                    anchor = Some(args[i + 1].clone());
                    i += 2;
                } else if (args[i] == "--kind" || args[i] == "-k") && i + 1 < args.len() {
                    kind = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    val_parts.push(args[i].clone());
                    i += 1;
                }
            }
            let val = val_parts.join(" ");
            Ok(Command::Set {
                key,
                val,
                anchor,
                kind,
            })
        }
        "relate" | "link" => {
            if args.len() < 5 {
                return Err(Error::Usage(
                    "Usage: agent-mem relate <source_key> <rel_type> <target_key>".to_string(),
                ));
            }
            Ok(Command::Relate {
                source: args[2].clone(),
                rel_type: args[3].clone(),
                target: args[4].clone(),
            })
        }
        "unrelate" | "unlink" => {
            if args.len() < 5 {
                return Err(Error::Usage(
                    "Usage: agent-mem unrelate <source_key> <rel_type> <target_key>".to_string(),
                ));
            }
            Ok(Command::Unrelate {
                source: args[2].clone(),
                rel_type: args[3].clone(),
                target: args[4].clone(),
            })
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
        "trust" | "untrust" => {
            if args.len() < 3 {
                return Err(Error::Usage(format!("Usage: agent-mem {cmd_str} <key>")));
            }
            Ok(Command::Trust {
                key: args[2].clone(),
                reviewed: cmd_str == "trust",
            })
        }
        "metadata" | "meta" => {
            if args.len() < 3 {
                return Err(Error::Usage("Usage: agent-mem metadata <key>".to_string()));
            }
            Ok(Command::Metadata {
                key: args[2].clone(),
            })
        }
        "history" => {
            if args.len() != 3 {
                return Err(Error::Usage("Usage: agent-mem history <key>".to_string()));
            }
            Ok(Command::History {
                key: args[2].clone(),
            })
        }
        "dump" | "list" | "ls" => Ok(Command::Dump),
        "clean" => {
            let dry_run = args.iter().skip(2).any(|a| a == "--dry-run" || a == "-n");
            Ok(Command::Clean { dry_run })
        }
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
        "context" => {
            let mut anchor = None;
            let mut topic = None;
            let mut limit = 20;
            let mut diff = false;
            let mut files = Vec::new();
            let mut i = 2;
            while i < args.len() {
                if (args[i] == "--anchor" || args[i] == "-a") && i + 1 < args.len() {
                    anchor = Some(args[i + 1].clone());
                    i += 2;
                } else if (args[i] == "--topic" || args[i] == "-t") && i + 1 < args.len() {
                    topic = Some(args[i + 1].clone());
                    i += 2;
                } else if (args[i] == "--limit" || args[i] == "-l" || args[i] == "-n")
                    && i + 1 < args.len()
                {
                    if let Ok(num) = args[i + 1].parse::<usize>() {
                        limit = num;
                    }
                    i += 2;
                } else if args[i] == "--diff" || args[i] == "-d" {
                    diff = true;
                    i += 1;
                } else if (args[i] == "--files" || args[i] == "-f") && i + 1 < args.len() {
                    for f in args[i + 1].split(',') {
                        let trimmed = f.trim();
                        if !trimmed.is_empty() {
                            files.push(trimmed.to_string());
                        }
                    }
                    i += 2;
                } else if !args[i].starts_with('-') && files.is_empty() {
                    for f in args[i].split(',') {
                        let trimmed = f.trim();
                        if !trimmed.is_empty() {
                            files.push(trimmed.to_string());
                        }
                    }
                    i += 1;
                } else {
                    i += 1;
                }
            }
            Ok(Command::Context {
                anchor,
                topic,
                limit,
                diff,
                files,
            })
        }
        "sync" => {
            let mut file = None;
            let mut export = false;
            let mut accept_conflicts = false;
            for arg in &args[2..] {
                if arg == "--export" || arg == "-e" {
                    export = true;
                } else if arg == "--accept-conflicts" {
                    accept_conflicts = true;
                } else if !arg.starts_with('-') && file.is_none() {
                    file = Some(arg.clone());
                }
            }
            if accept_conflicts && export {
                return Err(Error::Usage(
                    "--accept-conflicts cannot be combined with --export".into(),
                ));
            }
            if accept_conflicts {
                Ok(Command::SyncAcceptConflicts { file })
            } else {
                Ok(Command::Sync { file, export })
            }
        }
        "semantic" => match args.get(2).map(String::as_str) {
            Some("build" | "rebuild" | "enable") => Ok(Command::SemanticBuild),
            Some("status") => Ok(Command::SemanticStatus),
            Some("clear" | "disable") => Ok(Command::SemanticClear),
            _ => Err(Error::Usage(
                "Usage: agent-mem semantic [build|status|clear]".to_string(),
            )),
        },
        "mcp" => {
            if args.len() >= 3 && args[2] == "install" {
                let client = if args.len() >= 4 {
                    Some(args[3].clone())
                } else {
                    None
                };
                Ok(Command::McpInstall { client })
            } else if args.len() >= 3
                && (args[2] == "uninstall" || args[2] == "remove" || args[2] == "rm")
            {
                let client = if args.len() >= 4 {
                    Some(args[3].clone())
                } else {
                    None
                };
                Ok(Command::McpUninstall { client })
            } else {
                Ok(Command::Mcp {
                    read_only: args
                        .iter()
                        .skip(2)
                        .any(|arg| arg == "--read-only" || arg == "-r"),
                })
            }
        }
        "--mcp" => Ok(Command::Mcp { read_only: false }),
        "tui" | "ui" => Ok(Command::Tui),
        "doctor" => Ok(Command::Doctor),
        "projects" => {
            let prune = args
                .get(2)
                .map(|s| s.as_str())
                .is_some_and(|s| s == "prune" || s == "--prune" || s == "-p");
            Ok(Command::Projects { prune })
        }
        "hook" => {
            if args.len() < 3 {
                return Err(Error::Usage(
                    "Usage: agent-mem hook post-commit [--dry-run]".to_string(),
                ));
            }
            match args[2].as_str() {
                "post-commit" => {
                    let dry_run = args.iter().skip(3).any(|a| a == "--dry-run" || a == "-n");
                    Ok(Command::HookPostCommit { dry_run })
                }
                other => Err(Error::Usage(format!(
                    "Unknown hook subcommand '{}'. Usage: agent-mem hook post-commit [--dry-run]",
                    other
                ))),
            }
        }
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
        Command::Mcp { read_only } => {
            let server = crate::mcp::McpServer::new_with_mode(read_only);
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
        Command::McpUninstall { client } => {
            let results = crate::installer::uninstall_clients(client.as_deref())?;
            print_mcp_uninstall(&results);
            Ok(())
        }
        Command::Tui => {
            #[cfg(feature = "tui")]
            {
                if root.join(".agent-mem").join("mem.db").exists() {
                    crate::registry::ProjectRegistry::load()?.register(root)?;
                }
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
        Command::Projects { prune } => {
            let mut reg = crate::registry::ProjectRegistry::load()?;
            if root.join(".agent-mem").join("mem.db").exists() {
                reg.register(root)?;
            }
            if prune {
                let pruned = reg.prune()?;
                println!("Pruned {} dead project(s)", pruned);
            } else {
                crate::output::print_projects(&reg.projects);
            }
            Ok(())
        }
        Command::HookPostCommit { dry_run } => {
            if let Some(report) = crate::hook::run_post_commit(root, dry_run)? {
                crate::output::print_hook_report(&report);
            }
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
                Command::Set {
                    key,
                    val,
                    anchor,
                    kind,
                } => {
                    store.set_entry(&key, &val, anchor.as_deref(), kind.as_deref())?;
                    let rules_file = root.join(".agent-rules");
                    if rules_file.exists() {
                        store.export_to_file(&rules_file)?;
                    }
                    print_set(&key, anchor.as_deref(), kind.as_deref());
                }
                Command::Relate {
                    source,
                    rel_type,
                    target,
                } => {
                    store.relate(&source, &rel_type, &target)?;
                    let rules_file = root.join(".agent-rules");
                    if rules_file.exists() {
                        store.export_to_file(&rules_file)?;
                    }
                    print_relate(&source, &rel_type, &target);
                }
                Command::Unrelate {
                    source,
                    rel_type,
                    target,
                } => {
                    let unlinked = store.unrelate(&source, &rel_type, &target)?;
                    let rules_file = root.join(".agent-rules");
                    if rules_file.exists() {
                        store.export_to_file(&rules_file)?;
                    }
                    print_unrelate(&source, &rel_type, &target, unlinked);
                }
                Command::Del { key } => {
                    let deleted = store.del(&key)?;
                    let rules_file = root.join(".agent-rules");
                    if rules_file.exists() {
                        store.export_to_file(&rules_file)?;
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
                        store.export_to_file(&rules_file)?;
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
                        store.export_to_file(&rules_file)?;
                    }
                    print_unarchive(&key);
                }
                Command::Trust { key, reviewed } => {
                    let trust = if reviewed {
                        crate::store::TRUST_REVIEWED
                    } else {
                        crate::store::TRUST_UNTRUSTED
                    };
                    if !store.set_trust(&key, trust)? {
                        return Err(crate::error::Error::NotFound(key));
                    }
                    print_trust(&key, trust);
                }
                Command::Metadata { key } => {
                    let metadata = store
                        .metadata(&key)?
                        .ok_or_else(|| crate::error::Error::NotFound(key.clone()))?;
                    print_metadata(&metadata);
                }
                Command::History { key } => {
                    let history = store.history(&key)?;
                    if history.is_empty() {
                        return Err(crate::error::Error::NotFound(key));
                    }
                    print_history(&key, &history);
                }
                Command::Dump => {
                    let entries = store.dump()?;
                    print_dump(&entries);
                }
                Command::Clean { dry_run } => {
                    let report = store.clean_zombies(root, dry_run)?;
                    if !dry_run && !report.is_empty() {
                        let rules_file = root.join(".agent-rules");
                        if rules_file.exists() {
                            store.export_to_file(&rules_file)?;
                        }
                    }
                    print_clean(&report, dry_run);
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
                Command::Context {
                    anchor,
                    topic,
                    limit,
                    diff,
                    mut files,
                } => {
                    if diff {
                        let diff_files = crate::hook::get_diff_files(root);
                        for df in diff_files {
                            if !files.contains(&df) {
                                files.push(df);
                            }
                        }
                    }

                    let (rules, rels, sessions) = if !files.is_empty() {
                        store.context_for_files(&files, topic.as_deref(), limit)?
                    } else {
                        store.context_filtered(anchor.as_deref(), topic.as_deref(), limit)?
                    };
                    print_context_filtered(&rules, &rels, &sessions);
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
                Command::SyncAcceptConflicts { file } => {
                    let target_file = file.as_deref().unwrap_or(".agent-rules");
                    let file_path = if Path::new(target_file).is_absolute() {
                        Path::new(target_file).to_path_buf()
                    } else {
                        root.join(target_file)
                    };
                    let report = store.sync_with_file_accept_conflicts(&file_path)?;
                    print_sync(&report);
                }
                Command::SemanticBuild => {
                    #[cfg(feature = "semantic-local")]
                    {
                        let report = store.semantic_rebuild()?;
                        print_semantic_build(&report);
                    }
                    #[cfg(not(feature = "semantic-local"))]
                    {
                        return Err(Error::Usage(
                            "Local semantic search is not compiled in. Reinstall with '--features semantic-local'."
                                .to_string(),
                        ));
                    }
                }
                Command::SemanticStatus => {
                    #[cfg(feature = "semantic-local")]
                    {
                        let status = store.semantic_status()?;
                        print_semantic_status(&status);
                    }
                    #[cfg(not(feature = "semantic-local"))]
                    {
                        return Err(Error::Usage(
                            "Local semantic search is not compiled in. Reinstall with '--features semantic-local'."
                                .to_string(),
                        ));
                    }
                }
                Command::SemanticClear => {
                    #[cfg(feature = "semantic-local")]
                    {
                        let removed = store.semantic_clear()?;
                        print_semantic_clear(removed);
                    }
                    #[cfg(not(feature = "semantic-local"))]
                    {
                        return Err(Error::Usage(
                            "Local semantic search is not compiled in. Reinstall with '--features semantic-local'."
                                .to_string(),
                        ));
                    }
                }
                Command::Init { .. }
                | Command::Help
                | Command::Version
                | Command::Doctor
                | Command::Projects { .. }
                | Command::HookPostCommit { .. }
                | Command::Tui
                | Command::Mcp { .. }
                | Command::McpInstall { .. }
                | Command::McpUninstall { .. } => unreachable!(),
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
