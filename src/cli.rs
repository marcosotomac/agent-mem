use crate::error::{Error, Result};
use crate::init::{find_project_root, init_project};
use crate::output::*;
use crate::store::Store;
use std::env;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Init,
    Get { key: String },
    Set {
        key: String,
        val: String,
        anchor: Option<String>,
    },
    Del { key: String },
    Find { query: String },
    Dump,
    Context,
    SessionAdd { summary: String },
    SessionList,
    Mcp,
    McpInstall { client: Option<String> },
    Help,
    Version,
}

impl Command {
    pub fn is_write(&self) -> bool {
        matches!(self, Command::Set { .. } | Command::Del { .. } | Command::SessionAdd { .. })
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
        "init" => Ok(Command::Init),
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
                return Err(Error::Usage("Usage: agent-mem set <key> <value> [--anchor <path:line>]".to_string()));
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
                return Err(Error::Usage("Usage: agent-mem session [add <msg> | list]".to_string()));
            }
            match args[2].as_str() {
                "add" => {
                    if args.len() < 4 {
                        return Err(Error::Usage("Usage: agent-mem session add <summary>".to_string()));
                    }
                    let summary = args[3..].join(" ");
                    Ok(Command::SessionAdd { summary })
                }
                "list" | "ls" => Ok(Command::SessionList),
                other => Err(Error::Usage(format!("Unknown session subcommand '{}'. Usage: agent-mem session [add|list]", other))),
            }
        }
        "context" => Ok(Command::Context),
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
        "help" | "--help" | "-h" => Ok(Command::Help),
        "version" | "--version" | "-v" => Ok(Command::Version),
        unknown => Err(Error::Usage(format!("Unknown command '{}'. Run 'agent-mem --help' for usage.", unknown))),
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
        Command::Init => {
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
                println!("No supported clients detected. Run 'agent-mem mcp install [claude|cursor|antigravity]'.");
            } else {
                let is_tty = std::io::stdout().is_terminal();
                for r in results {
                    let status = if r.already_configured { "updated" } else { "configured" };
                    if is_tty {
                        println!("  \x1b[32m✔\x1b[0m \x1b[1m{}\x1b[0m {} in \x1b[2m{}\x1b[0m", status, r.client.display_name(), r.path.display());
                    } else {
                        println!("{} {} in {}", status, r.client.display_name(), r.path.display());
                    }
                }
            }
            Ok(())
        }
        _ => {
            let db_path = root.join(".agent-mem").join("mem.db");
            let mut store = Store::open(&db_path, cmd.is_write())?;

            match cmd {
                Command::Get { key } => {
                    if let Some(val) = store.get(&key)? {
                        print_get(&val);
                    } else {
                        return Err(crate::error::Error::NotFound(key));
                    }
                }
                Command::Set { key, val, anchor } => {
                    store.set_with_anchor(&key, &val, anchor.as_deref())?;
                    print_set(&key);
                }
                Command::Del { key } => {
                    let deleted = store.del(&key)?;
                    print_del(&key, deleted);
                }
                Command::Dump => {
                    let entries = store.dump()?;
                    print_dump(&entries);
                }
                Command::Find { query } => {
                    let entries = store.find(&query)?;
                    print_find(&entries);
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
                Command::Init
                | Command::Help
                | Command::Version
                | Command::Mcp
                | Command::McpInstall { .. } => unreachable!(),
            }
            Ok(())
        }
    }
}

pub fn run() -> Result<()> {
    let cmd = parse_args(env::args())?;
    let root = find_project_root();
    execute_command(cmd, &root)
}
