use crate::init::InitReport;
use std::io::{self, BufWriter, IsTerminal, Write};

pub fn print_get(val: &str) {
    print!("{}", val);
    let _ = io::stdout().flush();
}

pub fn print_set(key: &str) {
    if io::stdout().is_terminal() {
        println!("  \x1b[2msaved\x1b[0m    {}", key);
    }
}

pub fn print_del(key: &str, deleted: bool) {
    if io::stdout().is_terminal() {
        if deleted {
            println!("  \x1b[2mdeleted\x1b[0m  {}", key);
        } else {
            println!("  \x1b[2mnot found\x1b[0m {}", key);
        }
    }
}

pub fn print_dump(entries: &[(String, String)]) {
    let is_tty = io::stdout().is_terminal();
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    for (key, val) in entries {
        if is_tty {
            let _ = writeln!(out, "  \x1b[1m{:<16}\x1b[0m \x1b[38;5;250m{}\x1b[0m", key, val);
        } else {
            let _ = writeln!(out, "{}: {}", key, val);
        }
    }
    let _ = out.flush();
}

pub fn print_find(entries: &[(String, String)]) {
    print_dump(entries);
}

pub fn print_session_add() {
    if io::stdout().is_terminal() {
        println!("  \x1b[2mrecorded\x1b[0m session checkpoint");
    }
}

pub fn print_session_list(sessions: &[(i64, String)]) {
    let is_tty = io::stdout().is_terminal();
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    for (id, summary) in sessions {
        if is_tty {
            let _ = writeln!(out, "  \x1b[2m#{:<3}\x1b[0m {}", id, summary);
        } else {
            let _ = writeln!(out, "[#{}] {}", id, summary);
        }
    }
    let _ = out.flush();
}

pub fn print_context(rules: &[(String, String)], sessions: &[(i64, String)]) {
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    if !rules.is_empty() {
        let _ = writeln!(out, "== RULES ==");
        for (k, v) in rules {
            let _ = writeln!(out, "{}: {}", k, v);
        }
    }

    if !sessions.is_empty() {
        let _ = writeln!(out, "== SESSIONS ==");
        for (id, summary) in sessions {
            let _ = writeln!(out, "[#{}] {}", id, summary);
        }
    }
    let _ = out.flush();
}

pub fn print_init(report: &InitReport) {
    let is_tty = io::stdout().is_terminal();

    if is_tty {
        println!();
        println!("  \x1b[1magent-mem\x1b[0m \x1b[2minitialized in\x1b[0m {}", report.root.display());
        println!();
        println!("  \x1b[2mstore\x1b[0m    .agent-mem/mem.db \x1b[2m(sqlite wal)\x1b[0m");
        if report.gitignore_updated {
            println!("  \x1b[2mgit\x1b[0m      .agent-mem/ added to .gitignore");
        } else {
            println!("  \x1b[2mgit\x1b[0m      .agent-mem/ already ignored");
        }
        if report.hook_configured {
            println!("  \x1b[2mhook\x1b[0m     .git/hooks/post-commit active");
        }
        if report.created_rule_file {
            println!("  \x1b[2mtrigger\x1b[0m  created {}", report.trigger_target);
        } else if report.trigger_updated {
            println!("  \x1b[2mtrigger\x1b[0m  updated {}", report.trigger_target);
        } else {
            println!("  \x1b[2mtrigger\x1b[0m  active in {}", report.trigger_target);
        }
        println!();
    } else {
        println!("[agent-mem] Initialized in {}", report.root.display());
        println!("- Store: .agent-mem/mem.db (SQLite WAL)");
        if report.gitignore_updated {
            println!("- Git: added .agent-mem/ to .gitignore");
        }
        if report.hook_configured {
            println!("- Hook: .git/hooks/post-commit active");
        }
        if report.trigger_updated || report.created_rule_file {
            println!("- Trigger: updated {}", report.trigger_target);
        }
    }
}

pub fn print_help() {
    let is_tty = io::stdout().is_terminal();

    if is_tty {
        println!();
        println!("  \x1b[1magent-mem\x1b[0m \x1b[2m{}\x1b[0m", env!("CARGO_PKG_VERSION"));
        println!("  \x1b[2mLocal-first, sub-millisecond memory engine for AI coding agents.\x1b[0m");
        println!();
        println!("  \x1b[2mUSAGE\x1b[0m");
        println!("    $ agent-mem <command> [arguments]");
        println!();
        println!("  \x1b[2mCOMMANDS\x1b[0m");
        println!("    \x1b[1minit\x1b[0m                 Initialize isolated memory in repository");
        println!("    \x1b[1mget\x1b[0m  <key>           Retrieve raw value for key");
        println!("    \x1b[1mset\x1b[0m  <key> <val>     Record or update memory rule");
        println!("    \x1b[1mdel\x1b[0m  <key>           Delete a memory rule");
        println!("    \x1b[1mfind\x1b[0m <query>         Search rules via BM25 index");
        println!("    \x1b[1mdump\x1b[0m                 List all active rules");
        println!("    \x1b[1mcontext\x1b[0m              Export dense prompt block");
        println!("    \x1b[1msession add\x1b[0m  <msg>   Record session checkpoint");
        println!("    \x1b[1msession list\x1b[0m         Display recent checkpoints");
        println!();
    } else {
        println!(
            "agent-mem {}\nUsage: agent-mem <command> [args]\nCommands: init, get, set, del, find, dump, context, session add, session list",
            env!("CARGO_PKG_VERSION")
        );
    }
}
