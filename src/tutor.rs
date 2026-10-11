use crate::error::{Error, Result};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

const TUTOR_STEPS: &[TutorStep] = &[
    TutorStep {
        id: 1,
        title: "Welcome to agent-mem",
        description: "agent-mem is a local-first memory engine for AI coding agents. \
            It stores rules, decisions, gotchas, and patterns as a knowledge hypergraph \
            with semantic search. Everything runs locally — no servers, no cloud.",
        action: TutorAction::Explain,
        command: None,
        verify: None,
        commands: None,
    },
    TutorStep {
        id: 2,
        title: "Initialize a project",
        description: "Let's create an isolated memory for this tutorial. \
            We'll use a temporary directory so your real projects stay clean.",
        action: TutorAction::RunCommand,
        command: Some("agent-mem init"),
        verify: Some(|_| {
            PathBuf::from(".agent-mem/mem.db").exists()
        }),
        commands: None,
    },
    TutorStep {
        id: 3,
        title: "Store your first memory",
        description: "Memories are key-value pairs with types: rule, decision, gotcha, pattern. \
            The key prefix auto-detects the type (decision/*, gotcha/*, pattern/*, rule/*).",
        action: TutorAction::RunCommand,
        command: Some(r#"agent-mem set decision/auth "Use argon2id for password hashing""#),
        verify: Some(|_| {
            PathBuf::from(".agent-mem/mem.db").exists()
        }),
        commands: None,
    },
    TutorStep {
        id: 4,
        title: "Add more memories",
        description: "Let's add a few more memories to build a knowledge base.",
        action: TutorAction::RunCommands,
        commands: Some(&[
            r#"agent-mem set gotcha/timing "Compare HMAC signatures in constant time to prevent timing attacks""#,
            r#"agent-mem set pattern/retry "Exponential backoff with jitter for retries""#,
            r#"agent-mem set pattern/outbox "Persist domain events in transactional outbox""#,
        ]),
        verify: Some(|_| {
            PathBuf::from(".agent-mem/mem.db").exists()
        }),
        command: None,
    },
    TutorStep {
        id: 5,
        title: "Search with BM25 (keyword search)",
        description: "Fast full-text search across keys, values, and anchors. \
            Exact matches return instantly. Try searching for 'cursor' or 'argon2id'.",
        action: TutorAction::RunCommand,
        command: Some("agent-mem find argon2id"),
        verify: Some(|output| {
            output.contains("decision/auth") && output.contains("argon2id")
        }),
        commands: None,
    },
    TutorStep {
        id: 6,
        title: "Semantic search (natural language)",
        description: "Semantic search finds memories by meaning, not just keywords. \
            It uses a local multilingual embedding model. First, build the index:",
        action: TutorAction::RunCommand,
        command: Some("agent-mem semantic build"),
        verify: Some(|output| {
            output.contains("semantic index built") || output.contains("records")
        }),
        commands: None,
    },
    TutorStep {
        id: 7,
        title: "Try semantic search",
        description: "Now search with natural language — Spanish, English, or mixed. \
            The model understands paraphrases and intent.",
        action: TutorAction::RunCommand,
        command: Some("agent-mem find \"cómo hashear contraseñas seguro\""),
        verify: Some(|output| {
            output.contains("decision/auth") || output.contains("argon2id")
        }),
        commands: None,
    },
    TutorStep {
        id: 8,
        title: "Build the knowledge hypergraph",
        description: "Link memories with typed relations: mitigates, supersedes, depends_on, relates_to. \
            This creates a navigable graph of engineering knowledge.",
        action: TutorAction::RunCommand,
        command: Some("agent-mem relate decision/auth mitigates gotcha/timing"),
        verify: Some(|output| {
            output.contains("related") || output.contains("mitigates")
        }),
        commands: None,
    },
    TutorStep {
        id: 9,
        title: "Explore the graph",
        description: "See how memories connect. The graph shows relations and lets you traverse.",
        action: TutorAction::RunCommand,
        command: Some("agent-mem dump"),
        verify: Some(|output| {
            output.contains("decision/auth") && output.contains("gotcha/timing")
        }),
        commands: None,
    },
    TutorStep {
        id: 10,
        title: "Team sync with .agent-rules",
        description: "Share memories across your team via Git — no SQLite binary conflicts. \
            Export to .agent-rules (human-readable), commit, and teammates sync.",
        action: TutorAction::RunCommand,
        command: Some("agent-mem sync --export"),
        verify: Some(|_| {
            PathBuf::from(".agent-rules").exists()
        }),
        commands: None,
    },
    TutorStep {
        id: 11,
        title: "View the shared file",
        description: "The .agent-rules file is plain text — review it in PRs, resolve conflicts manually.",
        action: TutorAction::RunCommand,
        command: Some("cat .agent-rules"),
        verify: Some(|output| {
            output.contains("decision/auth") && output.contains("gotcha/timing")
        }),
        commands: None,
    },
    TutorStep {
        id: 12,
        title: "Context for AI agents (MCP)",
        description: "agent-mem runs as an MCP server. AI agents query it via mem_find, mem_context, mem_set, mem_manage. \
            Try: agent-mem mcp (runs server, Ctrl+C to stop).",
        action: TutorAction::Explain,
        command: None,
        verify: None,
        commands: None,
    },
    TutorStep {
        id: 13,
        title: "Install MCP in your editor",
        description: "One command configures 17+ AI clients (Cursor, VS Code, Zed, Claude Code, Codex, etc.):",
        action: TutorAction::RunCommand,
        command: Some("agent-mem mcp install"),
        verify: None,
        commands: None,
    },
    TutorStep {
        id: 14,
        title: "Doctor: health check",
        description: "Verify your setup: storage, git hooks, MCP clients, project registry.",
        action: TutorAction::RunCommand,
        command: Some("agent-mem doctor"),
        verify: None,
        commands: None,
    },
    TutorStep {
        id: 15,
        title: "You're ready!",
        description: "Key concepts learned:\n\
            • agent-mem init — isolated project memory\n\
            • agent-mem set — store rules/decisions/gotchas/patterns\n\
            • agent-mem find — BM25 + semantic hybrid search\n\
            • agent-mem relate — build knowledge hypergraph\n\
            • agent-mem sync — team sharing via Git\n\
            • agent-mem mcp — AI agent integration\n\
            \nNext steps:\n\
            • Run 'agent-mem tutor 3' to replay from step 3\n\
            • Run 'agent-mem init' in your real project\n\
            • Add memories as you code: 'agent-mem set decision/...'\n\
            • Share with team: commit .agent-rules\n\
            \nDocs: https://github.com/marcosotomac/agent-mem",
        action: TutorAction::Explain,
        command: None,
        verify: None,
        commands: None,
    },
];

struct TutorStep {
    id: usize,
    title: &'static str,
    description: &'static str,
    action: TutorAction,
    command: Option<&'static str>,
    commands: Option<&'static [&'static str]>,
    verify: Option<fn(&str) -> bool>,
}

impl Default for TutorStep {
    fn default() -> Self {
        Self {
            id: 0,
            title: "",
            description: "",
            action: TutorAction::Explain,
            command: None,
            commands: None,
            verify: None,
        }
    }
}

enum TutorAction {
    Explain,
    RunCommand,
    RunCommands,
}

pub fn run_tutor(_root: &std::path::Path, start_step: Option<usize>, non_interactive: bool) -> Result<()> {
    let temp_dir = env::temp_dir().join(format!("agent-mem-tutor-{}", std::process::id()));
    fs::create_dir_all(&temp_dir)?;

    let original_dir = env::current_dir()?;
    env::set_current_dir(&temp_dir)?;

    println!("\x1b[2J\x1b[H"); // Clear screen
    print_banner();

    let start = start_step.unwrap_or(1).max(1).min(TUTOR_STEPS.len());

    for step in &TUTOR_STEPS[start - 1..] {
        print_step_header(step);

        match step.action {
            TutorAction::Explain => {
                println!("{}", step.description);
            }
            TutorAction::RunCommand => {
                if let Some(cmd) = step.command {
                    println!("\x1b[38;5;240m$ {}\x1b[0m", cmd);
                    if !non_interactive {
                        prompt_continue()?;
                    }
                    let output = run_command(cmd)?;
                    println!("{}", output.trim_end());

                    if let Some(verify) = step.verify {
                        if !verify(&output) {
                            println!("\x1b[38;5;203m⚠ Verification failed — but continuing...\x1b[0m");
                        } else {
                            println!("\x1b[38;5;150m✓ Verified\x1b[0m");
                        }
                    }
                }
            }
            TutorAction::RunCommands => {
                if let Some(cmds) = step.commands {
                    for cmd in cmds {
                        println!("\x1b[38;5;240m$ {}\x1b[0m", cmd);
                        if !non_interactive {
                            prompt_continue()?;
                        }
                        let output = run_command(cmd)?;
                        println!("{}", output.trim_end());
                    }
                    if let Some(verify) = step.verify {
                        if let Some(last_cmd) = cmds.last() {
                            let output = run_command(last_cmd)?;
                            if !verify(&output) {
                                println!("\x1b[38;5;203m⚠ Verification failed — but continuing...\x1b[0m");
                            } else {
                                println!("\x1b[38;5;150m✓ Verified\x1b[0m");
                            }
                        }
                    }
                }
            }
        }

        if !non_interactive && step.id < TUTOR_STEPS.len() {
            println!();
            prompt_next_step()?;
        }
    }

    env::set_current_dir(&original_dir)?;
    let _ = fs::remove_dir_all(&temp_dir);

    print_completion_summary();
    Ok(())
}

fn print_completion_summary() {
    println!();
    println!("[1;37m╔══════════════════════════════════════════════════════════╗");
    println!("║  [38;5;150m✓ Tutorial completed![1;37m                                ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  [38;5;245mQuick reference:[1;37m                                          ║");
    println!("║  [38;5;150magent-mem init[0m          Start a new project memory     ║");
    println!("║  [38;5;150magent-mem set k v[0m       Save a rule/decision/gotcha   ║");
    println!("║  [38;5;150magent-mem find q[0m        Search (BM25 + semantic)       ║");
    println!("║  [38;5;150magent-mem relate a r b[0m   Link memories                  ║");
    println!("║  [38;5;150magent-mem sync -e[0m       Export to .agent-rules         ║");
    println!("║  [38;5;150magent-mem mcp install[0m   Connect AI editors              ║");
    println!("║  [38;5;150magent-mem doctor[0m        Health check                   ║");
    println!("║  [38;5;150magent-mem tui[0m           Visual explorer                ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  [38;5;245mDocs: https://github.com/marcosotomac/agent-mem[1;37m         ║");
    println!("╚══════════════════════════════════════════════════════════╝[0m");
    println!();
}

fn print_banner() {
    println!("\x1b[1;37m╔══════════════════════════════════════════════════════════╗");
    println!("║           \x1b[38;5;150magent-mem interactive tutor\x1b[1;37m             ║");
    println!("║        Learn the basics in ~5 minutes                    ║");
    println!("╚══════════════════════════════════════════════════════════╝\x1b[0m");
    println!();
    println!("This tutorial runs in a temporary directory:");
    println!("  \x1b[38;5;240m{}\x1b[0m", env::temp_dir().join(format!("agent-mem-tutor-{}", std::process::id())).display());
    println!();
    println!("Press \x1b[1;37mEnter\x1b[0m to continue each step, \x1b[1;37mCtrl+C\x1b[0m to quit.");
    println!();
}

fn print_step_header(step: &TutorStep) {
    println!("\x1b[1;37m┌─ Step {} / {} ────────────────────────────────────────┐\x1b[0m", step.id, TUTOR_STEPS.len());
    println!("\x1b[1;37m│\x1b[0m \x1b[1;37m{}\x1b[0m", step.title);
    println!("\x1b[1;37m└──────────────────────────────────────────────────────┘\x1b[0m");
    println!();
}

fn prompt_continue() -> Result<()> {
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(())
}

fn prompt_next_step() -> Result<()> {
    print!("  \x1b[38;5;240m→ Press Enter for next step, 'q' to quit, 'r' to repeat\x1b[0m ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    match input.trim().to_lowercase().as_str() {
        "q" | "quit" => {
            println!("\n\x1b[38;5;245mTutorial ended. Run 'agent-mem tutor' to restart.\x1b[0m\n");
            std::process::exit(0);
        }
        "r" | "repeat" => {
            // We can't easily repeat in this structure, just continue
        }
        _ => {}
    }
    Ok(())
}

fn run_command(cmd: &str) -> Result<String> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let program = parts[0];
    let args = &parts[1..];

    let output = Command::new(program)
        .args(args)
        .output()?;

    let mut result = String::new();
    if !output.stdout.is_empty() {
        result.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&String::from_utf8_lossy(&output.stderr));
    }

    if !output.status.success() && result.is_empty() {
        result.push_str(&format!("Command failed with exit code: {:?}", output.status.code()));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tutor_steps_are_valid() {
        assert!(!TUTOR_STEPS.is_empty());
        for (i, step) in TUTOR_STEPS.iter().enumerate() {
            assert_eq!(step.id, i + 1);
            assert!(!step.title.is_empty());
            assert!(!step.description.is_empty());
        }
    }
}