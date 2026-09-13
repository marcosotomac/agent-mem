use crate::init::InitReport;
use std::io::{self, BufWriter, IsTerminal, Write};

// shadcn / geist color tokens (256-color ANSI for terminal fidelity)
const ACCENT: &str = "\x1b[1;37m"; // Pure white bold (headings, keys)
const MUTED: &str = "\x1b[38;5;245m"; // Zinc-400 (secondary text, labels)
const SUBTLE: &str = "\x1b[38;5;240m"; // Zinc-700 (dots, borders, numbers)
const BODY: &str = "\x1b[38;5;252m"; // Zinc-200 (readable body text)
const EMERALD: &str = "\x1b[38;5;150m"; // Soft emerald green (checks, success)
const AMBER: &str = "\x1b[38;5;216m"; // Soft amber (warnings, deletions)
const RESET: &str = "\x1b[0m";

pub fn print_get(val: &str) {
    if io::stdout().is_terminal() {
        if val.ends_with('\n') {
            print!("{}", val);
        } else {
            println!("{}", val);
        }
    } else {
        print!("{}", val);
        let _ = io::stdout().flush();
    }
}

pub fn print_set(key: &str, anchor: Option<&str>, kind: Option<&str>) {
    let effective_kind = kind.unwrap_or_else(|| crate::store::infer_kind(key));
    let kind_badge = if effective_kind != "rule" {
        format!("[{}] ", effective_kind)
    } else {
        String::new()
    };
    if io::stdout().is_terminal() {
        match anchor {
            Some(a) => println!(
                "  {EMERALD}✓{RESET}  {MUTED}saved{RESET}  {MUTED}{kind_badge}{RESET}{ACCENT}{key}{RESET}  {SUBTLE}·{RESET}  {MUTED}{a}{RESET}"
            ),
            None => println!(
                "  {EMERALD}✓{RESET}  {MUTED}saved{RESET}  {MUTED}{kind_badge}{RESET}{ACCENT}{key}{RESET}"
            ),
        }
    } else {
        match anchor {
            Some(a) => println!("saved {}{key} ({a})", kind_badge),
            None => println!("saved {}{key}", kind_badge),
        }
    }
}

pub fn print_relate(source: &str, rel_type: &str, target: &str) {
    if io::stdout().is_terminal() {
        println!(
            "  {EMERALD}✓{RESET}  {MUTED}linked{RESET}  {ACCENT}{source}{RESET} {SUBTLE}-> {rel_type} ->{RESET} {ACCENT}{target}{RESET}"
        );
    } else {
        println!("linked {} -> {} -> {}", source, rel_type, target);
    }
}

pub fn print_unrelate(source: &str, rel_type: &str, target: &str, unlinked: bool) {
    if io::stdout().is_terminal() {
        if unlinked {
            println!(
                "  {AMBER}-{RESET}  {MUTED}unlinked{RESET}  {ACCENT}{source}{RESET} {SUBTLE}-> {rel_type} ->{RESET} {ACCENT}{target}{RESET}"
            );
        } else {
            println!(
                "  {SUBTLE}·{RESET}  {MUTED}not found{RESET}  {ACCENT}{source}{RESET} {SUBTLE}-> {rel_type} ->{RESET} {ACCENT}{target}{RESET}"
            );
        }
    } else if unlinked {
        println!("unlinked {} -> {} -> {}", source, rel_type, target);
    } else {
        println!(
            "relation not found {} -> {} -> {}",
            source, rel_type, target
        );
    }
}

pub fn print_del(key: &str, deleted: bool) {
    if io::stdout().is_terminal() {
        if deleted {
            println!("  {AMBER}-{RESET}  {MUTED}deleted{RESET}  {ACCENT}{key}{RESET}");
        } else {
            println!("  {SUBTLE}·{RESET}  {MUTED}not found{RESET}  {ACCENT}{key}{RESET}");
        }
    } else if deleted {
        println!("deleted {}", key);
    } else {
        println!("not found {}", key);
    }
}

pub fn print_get_record(record: &crate::store::RuleRecord) {
    if io::stdout().is_terminal() {
        if record.is_archived() {
            let reason = record
                .archive_reason
                .as_deref()
                .map(|r| format!("  {AMBER}(reason: {}){RESET}", r))
                .unwrap_or_default();
            println!(
                "  {AMBER}⊘ [archived]{RESET} {ACCENT}{}{RESET}  {SUBTLE}·{RESET}  {MUTED}{}{RESET}{}",
                record.key, record.val, reason
            );
        } else {
            print_get(&record.val);
        }
    } else if record.is_archived() {
        let reason = record
            .archive_reason
            .as_deref()
            .map(|r| format!(" (reason: {})", r))
            .unwrap_or_default();
        println!("[archived] {}: {}{}", record.key, record.val, reason);
    } else {
        print_get(&record.val);
    }
}

pub fn print_archive(key: &str, reason: Option<&str>) {
    if io::stdout().is_terminal() {
        match reason {
            Some(r) => println!(
                "  {AMBER}⊘{RESET}  {MUTED}archived{RESET}  {ACCENT}{key}{RESET}  {SUBTLE}·{RESET}  {AMBER}{r}{RESET}"
            ),
            None => println!("  {AMBER}⊘{RESET}  {MUTED}archived{RESET}  {ACCENT}{key}{RESET}"),
        }
    } else {
        match reason {
            Some(r) => println!("archived {} ({})", key, r),
            None => println!("archived {}", key),
        }
    }
}

pub fn print_unarchive(key: &str) {
    if io::stdout().is_terminal() {
        println!("  {EMERALD}✓{RESET}  {MUTED}reactivated{RESET}  {ACCENT}{key}{RESET}");
    } else {
        println!("reactivated {}", key);
    }
}

pub fn print_dump(entries: &[(String, String, Option<String>)]) {
    let is_tty = io::stdout().is_terminal();
    if entries.is_empty() {
        if is_tty {
            println!(
                "  {SUBTLE}◇{RESET}  {MUTED}no memories recorded yet. Run 'agent-mem set <key> <val>' to add one.{RESET}"
            );
        }
        return;
    }

    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let max_key_len = entries
        .iter()
        .map(|(k, _, _)| k.len())
        .max()
        .unwrap_or(12)
        .clamp(12, 36);

    for (key, val, anchor) in entries {
        match (is_tty, anchor) {
            (true, Some(a)) => {
                let _ = writeln!(
                    out,
                    "  {ACCENT}{:<width$}{RESET}  {SUBTLE}·{RESET}  {BODY}{}{RESET}  {MUTED}{}{RESET}",
                    key,
                    val,
                    a,
                    width = max_key_len
                );
            }
            (true, None) => {
                let _ = writeln!(
                    out,
                    "  {ACCENT}{:<width$}{RESET}  {SUBTLE}·{RESET}  {BODY}{}{RESET}",
                    key,
                    val,
                    width = max_key_len
                );
            }
            (false, Some(a)) => {
                let _ = writeln!(out, "{}: {} ({})", key, val, a);
            }
            (false, None) => {
                let _ = writeln!(out, "{}: {}", key, val);
            }
        }
    }
    let _ = out.flush();
}

pub fn print_find(query: &str, entries: &[crate::store::RuleRecord]) {
    if entries.is_empty() {
        if io::stdout().is_terminal() {
            println!(
                "  {SUBTLE}◇{RESET}  {MUTED}no memories found matching{RESET} '{}'",
                query
            );
        }
        return;
    }

    let is_tty = io::stdout().is_terminal();
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let max_key_len = entries
        .iter()
        .map(|r| r.key.len())
        .max()
        .unwrap_or(12)
        .clamp(12, 36);

    for record in entries {
        let kind_tag = if record.kind != "rule" {
            format!("[{}] ", record.kind)
        } else {
            String::new()
        };

        if record.is_archived() {
            let reason = record
                .archive_reason
                .as_deref()
                .map(|r| format!(" (reason: {})", r))
                .unwrap_or_default();
            if is_tty {
                let _ = writeln!(
                    out,
                    "  {AMBER}⊘ [archived]{RESET} {MUTED}{}{RESET}{ACCENT}{:<width$}{RESET}  {SUBTLE}·{RESET}  {MUTED}{}{RESET}  {AMBER}{}{RESET}",
                    kind_tag,
                    record.key,
                    record.val,
                    reason,
                    width = max_key_len
                );
            } else {
                let _ = writeln!(
                    out,
                    "[archived] {}{}: {}{}",
                    kind_tag, record.key, record.val, reason
                );
            }
        } else {
            match (is_tty, record.anchor.as_deref()) {
                (true, Some(a)) => {
                    let _ = writeln!(
                        out,
                        "  {MUTED}{}{RESET}{ACCENT}{:<width$}{RESET}  {SUBTLE}·{RESET}  {BODY}{}{RESET}  {MUTED}{}{RESET}",
                        kind_tag,
                        record.key,
                        record.val,
                        a,
                        width = max_key_len
                    );
                }
                (true, None) => {
                    let _ = writeln!(
                        out,
                        "  {MUTED}{}{RESET}{ACCENT}{:<width$}{RESET}  {SUBTLE}·{RESET}  {BODY}{}{RESET}",
                        kind_tag,
                        record.key,
                        record.val,
                        width = max_key_len
                    );
                }
                (false, Some(a)) => {
                    let _ = writeln!(out, "{}{}: {} ({})", kind_tag, record.key, record.val, a);
                }
                (false, None) => {
                    let _ = writeln!(out, "{}{}: {}", kind_tag, record.key, record.val);
                }
            }
        }
    }
    let _ = out.flush();
}

pub fn print_session_add(id: i64) {
    if io::stdout().is_terminal() {
        println!("  {EMERALD}✓{RESET}  {MUTED}checkpoint{RESET}  {ACCENT}#{id}{RESET}");
    } else {
        println!("recorded #{}", id);
    }
}

pub fn print_session_list(sessions: &[(i64, String)]) {
    let is_tty = io::stdout().is_terminal();
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    for (id, summary) in sessions {
        if is_tty {
            let _ = writeln!(out, "  {SUBTLE}#{:<3}{RESET}  {BODY}{}{RESET}", id, summary);
        } else {
            let _ = writeln!(out, "[#{}] {}", id, summary);
        }
    }
    let _ = out.flush();
}

pub fn print_context_filtered(
    rules: &[crate::store::RuleRecord],
    relations: &[crate::store::RelationRecord],
    sessions: &[(i64, String)],
) {
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    if !rules.is_empty() {
        let _ = writeln!(out, "== RULES ==");
        for r in rules {
            let kind_badge = if r.kind != "rule" {
                format!("[{}] ", r.kind)
            } else {
                String::new()
            };
            if let Some(anchor) = &r.anchor {
                let _ = writeln!(out, "{}{}: {} ({})", kind_badge, r.key, r.val, anchor);
            } else {
                let _ = writeln!(out, "{}{}: {}", kind_badge, r.key, r.val);
            }
        }
    }

    if !relations.is_empty() {
        let _ = writeln!(out, "== RELATIONS ==");
        for rel in relations {
            let _ = writeln!(
                out,
                "{} -> {} -> {}",
                rel.source_key, rel.rel_type, rel.target_key
            );
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

pub fn print_context(rules: &[(String, String, Option<String>)], sessions: &[(i64, String)]) {
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    if !rules.is_empty() {
        let _ = writeln!(out, "== RULES ==");
        for (k, v, a) in rules {
            if let Some(anchor) = a {
                let _ = writeln!(out, "{}: {} ({})", k, v, anchor);
            } else {
                let _ = writeln!(out, "{}: {}", k, v);
            }
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
        println!(
            "  {ACCENT}agent-mem{RESET} {MUTED}{}{RESET}  {SUBTLE}·{RESET}  {MUTED}initialized{RESET}",
            env!("CARGO_PKG_VERSION")
        );
        println!("  {SUBTLE}{}{RESET}", report.root.display());
        println!();
        println!("  {MUTED}store{RESET}     .agent-mem/mem.db  {SUBTLE}(sqlite wal){RESET}");
        if report.gitignore_updated {
            println!("  {MUTED}git{RESET}       .agent-mem/ added to .gitignore");
        }
        if report.gitattributes_updated {
            println!(
                "  {MUTED}git{RESET}       .agent-rules union merge configured in .gitattributes"
            );
        }
        if report.hook_configured {
            println!(
                "  {MUTED}hooks{RESET}     post-commit active  {SUBTLE}(auto-sessions){RESET}"
            );
        }
        if report.post_merge_configured {
            println!("  {MUTED}hooks{RESET}     post-merge active  {SUBTLE}(auto-sync){RESET}");
        }
        if report.post_checkout_configured {
            println!("  {MUTED}hooks{RESET}     post-checkout active  {SUBTLE}(auto-sync){RESET}");
        }
        if report.post_rewrite_configured {
            println!(
                "  {MUTED}hooks{RESET}     post-rewrite active  {SUBTLE}(auto-sync on rebase){RESET}"
            );
        }
        if report.rules_file_created {
            println!(
                "  {MUTED}sync{RESET}      created .agent-rules  {SUBTLE}(team git sync){RESET}"
            );
        }
        if report.created_rule_file {
            println!(
                "  {MUTED}protocol{RESET}  created {}",
                report.trigger_target
            );
        } else if report.trigger_updated {
            println!(
                "  {MUTED}protocol{RESET}  updated {}",
                report.trigger_target
            );
        } else {
            println!(
                "  {MUTED}protocol{RESET}  active in {}",
                report.trigger_target
            );
        }
        for client_res in &report.configured_clients {
            if client_res.already_configured {
                println!(
                    "  {MUTED}mcp{RESET}       {}  {SUBTLE}(already configured){RESET}",
                    client_res.client.display_name()
                );
            } else {
                println!(
                    "  {EMERALD}✓{RESET}  {MUTED}mcp{RESET}       {}  {SUBTLE}(auto-configured){RESET}",
                    client_res.client.display_name()
                );
            }
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
        if report.post_merge_configured {
            println!("- Hook: .git/hooks/post-merge active (auto-sync)");
        }
        if report.post_checkout_configured {
            println!("- Hook: .git/hooks/post-checkout active (auto-sync)");
        }
        if report.post_rewrite_configured {
            println!("- Hook: .git/hooks/post-rewrite active (auto-sync on rebase)");
        }
        if report.rules_file_created {
            println!("- Sync: created .agent-rules (team git sync)");
        }
        println!("- Protocol: updated {}", report.trigger_target);
        for client_res in &report.configured_clients {
            if client_res.already_configured {
                println!(
                    "- MCP: {} (already configured)",
                    client_res.client.display_name()
                );
            } else {
                println!(
                    "- MCP: {} auto-configured",
                    client_res.client.display_name()
                );
            }
        }
    }
}

pub fn print_sync(report: &crate::store::SyncReport) {
    let is_tty = io::stdout().is_terminal();
    let file_name = report
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".agent-rules");
    if is_tty {
        if report.file_created {
            println!(
                "  {EMERALD}✓{RESET}  {MUTED}created{RESET}  {ACCENT}{file_name}{RESET}  {SUBTLE}·{RESET}  {MUTED}{} rules exported{RESET}",
                report.total
            );
        } else if report.file_updated {
            println!(
                "  {EMERALD}✓{RESET}  {MUTED}exported{RESET}  {ACCENT}{file_name}{RESET}  {SUBTLE}·{RESET}  {MUTED}{} rules written{RESET}",
                report.total
            );
        } else {
            println!(
                "  {EMERALD}✓{RESET}  {MUTED}synchronized{RESET}  {ACCENT}{file_name}{RESET}  {SUBTLE}·{RESET}  {MUTED}{} rules active{RESET}",
                report.total
            );
        }
    } else if report.file_created {
        println!("created {} ({} rules)", file_name, report.total);
    } else if report.file_updated {
        println!("exported {} ({} rules)", file_name, report.total);
    } else {
        println!("sync: {} rules in {}", report.total, file_name);
    }
}

pub fn print_help() {
    let is_tty = io::stdout().is_terminal();

    if is_tty {
        println!();
        println!(
            "  {ACCENT}agent-mem{RESET} {MUTED}{}{RESET}",
            env!("CARGO_PKG_VERSION")
        );
        println!(
            "  {MUTED}Local-first, sub-millisecond knowledge hypergraph for AI coding agents.{RESET}"
        );
        println!();
        println!("  {MUTED}Usage{RESET}");
        println!("    {SUBTLE}${RESET} agent-mem {MUTED}<command> [arguments]{RESET}");
        println!();
        println!("  {MUTED}Commands{RESET}");
        println!(
            "    {ACCENT}init{RESET}                    Initialize isolated memory in repository"
        );
        println!(
            "    {ACCENT}get{RESET}     {MUTED}<key>{RESET}           Retrieve raw value for key"
        );
        println!(
            "    {ACCENT}set{RESET}     {MUTED}<key> <val> [--anchor <f:l>] [--kind <k>]{RESET} Record memory"
        );
        println!(
            "    {ACCENT}relate{RESET}  {MUTED}<src> <rel> <tgt>{RESET} Link memories in hypergraph"
        );
        println!(
            "    {ACCENT}unrelate{RESET} {MUTED}<src> <rel> <tgt>{RESET} Remove hypergraph relation"
        );
        println!("    {ACCENT}del{RESET}     {MUTED}<key>{RESET}           Delete a memory rule");
        println!(
            "    {ACCENT}archive{RESET} {MUTED}<key> [--reason <msg>]{RESET} Deprecate an obsolete rule"
        );
        println!(
            "    {ACCENT}unarchive{RESET} {MUTED}<key>{RESET}       Reactivate an archived rule"
        );
        println!(
            "    {ACCENT}find{RESET}    {MUTED}<query>{RESET}         Search rules via BM25 index"
        );
        println!("    {ACCENT}dump{RESET}                    List all active rules");
        println!(
            "    {ACCENT}sync{RESET}    {MUTED}[file] [--export]{RESET} Synchronize team rules (.agent-rules)"
        );
        println!(
            "    {ACCENT}context{RESET} {MUTED}[--anchor <a>] [--topic <t>] [--limit <n>]{RESET} Export dense context"
        );
        println!(
            "    {ACCENT}session{RESET} {MUTED}add <msg>{RESET}       Record session checkpoint"
        );
        println!(
            "    {ACCENT}session{RESET} {MUTED}list{RESET}            Display recent checkpoints"
        );
        println!(
            "    {ACCENT}mcp{RESET}                     Start Model Context Protocol stdio server"
        );
        println!(
            "    {ACCENT}mcp{RESET}     {MUTED}install [client]{RESET} Configure AI editors (Windsurf, Cursor, VS Code, Zed, etc.)"
        );
        println!(
            "    {ACCENT}doctor{RESET}                  Verify system health, SQLite WAL, git hooks, and MCP"
        );
        println!(
            "    {ACCENT}projects{RESET} {MUTED}[prune]{RESET}        List canonical project registry & anti-collision status"
        );
        println!("    {ACCENT}tui{RESET}                     Launch interactive terminal explorer");
        println!();
    } else {
        println!(
            "agent-mem {}\nUsage: agent-mem <command> [args]\nCommands: init, get, set, relate, unrelate, del, archive, unarchive, find, dump, context, session add, session list, sync, mcp, mcp install, doctor, projects, tui",
            env!("CARGO_PKG_VERSION")
        );
    }
}

pub fn print_doctor(
    store_stats: Option<&crate::store::StoreStats>,
    git_stats: &crate::init::GitDoctorReport,
    clients: &[crate::installer::ClientStatus],
) {
    let is_tty = io::stdout().is_terminal();

    if is_tty {
        println!();
        println!(
            "  {ACCENT}agent-mem doctor{RESET}  {SUBTLE}·{RESET}  {MUTED}system health check{RESET}"
        );
        println!();

        // Storage
        println!("  {MUTED}Storage{RESET}");
        if let Some(stats) = store_stats {
            let rules_summary = if stats.archived_rules_count > 0 {
                format!(
                    "{} active ({} archived)",
                    stats.active_rules_count, stats.archived_rules_count
                )
            } else {
                format!("{} rules", stats.rules_count)
            };
            println!(
                "    {EMERALD}✓{RESET}  {MUTED}database{RESET}  {ACCENT}{}{RESET}  {SUBTLE}·{RESET}  {MUTED}{} mode  ·  {}, {} sessions{RESET}",
                stats.db_path.display(),
                stats.journal_mode,
                rules_summary,
                stats.sessions_count
            );
            println!(
                "    {EMERALD}✓{RESET}  {MUTED}full-text{RESET} {ACCENT}FTS5 BM25{RESET}  {SUBTLE}·{RESET}  {MUTED}porter unicode61 tokenizer active{RESET}"
            );
        } else {
            println!(
                "    {AMBER}!{RESET}  {MUTED}database{RESET}  {ACCENT}not initialized{RESET}  {SUBTLE}·{RESET}  {MUTED}run 'agent-mem init' to initialize{RESET}"
            );
        }
        println!();

        // Git & Team Sync
        println!("  {MUTED}Git & Team Sync{RESET}");
        if git_stats.is_git_repo {
            println!(
                "    {EMERALD}✓{RESET}  {MUTED}repo{RESET}      {ACCENT}{}{RESET}",
                git_stats.root.display()
            );
            if git_stats.gitignore_active {
                println!(
                    "    {EMERALD}✓{RESET}  {MUTED}ignore{RESET}    {ACCENT}.agent-mem/{RESET}  {SUBTLE}·{RESET}  {MUTED}ignored in .gitignore{RESET}"
                );
            } else {
                println!(
                    "    {AMBER}!{RESET}  {MUTED}ignore{RESET}    {ACCENT}.agent-mem/{RESET}  {SUBTLE}·{RESET}  {MUTED}not in .gitignore{RESET}"
                );
            }
            if git_stats.gitattributes_active {
                println!(
                    "    {EMERALD}✓{RESET}  {MUTED}sync{RESET}      {ACCENT}.agent-rules{RESET}  {SUBTLE}·{RESET}  {MUTED}merge=union configured{RESET}"
                );
            } else {
                println!(
                    "    {AMBER}!{RESET}  {MUTED}sync{RESET}      {ACCENT}.agent-rules{RESET}  {SUBTLE}·{RESET}  {MUTED}union merge not configured{RESET}"
                );
            }
            let all_hooks_active = git_stats.post_commit_active
                && git_stats.post_merge_active
                && git_stats.post_checkout_active
                && git_stats.post_rewrite_active;
            if all_hooks_active {
                println!(
                    "    {EMERALD}✓{RESET}  {MUTED}hooks{RESET}     {ACCENT}active{RESET}  {SUBTLE}·{RESET}  {MUTED}post-commit, post-merge, post-checkout, post-rewrite{RESET}"
                );
            } else if git_stats.post_commit_active {
                println!(
                    "    {EMERALD}✓{RESET}  {MUTED}hooks{RESET}     {ACCENT}partial{RESET}  {SUBTLE}·{RESET}  {MUTED}post-commit active{RESET}"
                );
            } else {
                println!(
                    "    {SUBTLE}·{RESET}  {MUTED}hooks{RESET}     {MUTED}none installed  ·  run 'agent-mem init'{RESET}"
                );
            }
            if git_stats.rules_file_exists {
                println!(
                    "    {EMERALD}✓{RESET}  {MUTED}rules{RESET}     {ACCENT}.agent-rules{RESET}  {SUBTLE}·{RESET}  {MUTED}{} rules active{RESET}",
                    git_stats.rules_count
                );
            }
        } else {
            println!(
                "    {SUBTLE}·{RESET}  {MUTED}git{RESET}       {MUTED}not a git repository{RESET}"
            );
        }
        println!();

        // MCP Clients
        println!("  {MUTED}MCP Clients{RESET}");
        let mut shown_any = false;
        for client in clients {
            if !client.installed && !client.configured {
                continue;
            }
            shown_any = true;
            let c_name = client.client.display_name();
            if client.configured {
                let p_str = client
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                println!(
                    "    {EMERALD}✓{RESET}  {ACCENT}{c_name:<18}{RESET}  {MUTED}configured{RESET}  {SUBTLE}·{RESET}  {MUTED}{p_str}{RESET}"
                );
            } else {
                let name = client.client.cli_id();
                println!(
                    "    {AMBER}!{RESET}  {ACCENT}{c_name:<18}{RESET}  {AMBER}detected (not configured){RESET}  {SUBTLE}·{RESET}  {MUTED}run 'agent-mem mcp install {name}'{RESET}"
                );
            }
        }
        if !shown_any {
            println!(
                "    {SUBTLE}·{RESET}  {MUTED}no supported AI clients detected on system{RESET}"
            );
        }
        println!();
    } else {
        println!("=== AGENT-MEM DOCTOR ===");
        if let Some(stats) = store_stats {
            println!(
                "Storage: {} (mode: {}, rules: {} [active: {}, archived: {}], sessions: {})",
                stats.db_path.display(),
                stats.journal_mode,
                stats.rules_count,
                stats.active_rules_count,
                stats.archived_rules_count,
                stats.sessions_count
            );
        } else {
            println!("Storage: not initialized");
        }
        println!(
            "Git: repo={}, ignore={}, attributes={}, post-commit={}, post-merge={}, post-checkout={}, post-rewrite={}, rules={}",
            git_stats.is_git_repo,
            git_stats.gitignore_active,
            git_stats.gitattributes_active,
            git_stats.post_commit_active,
            git_stats.post_merge_active,
            git_stats.post_checkout_active,
            git_stats.post_rewrite_active,
            git_stats.rules_count
        );
        for client in clients {
            if client.installed || client.configured {
                println!(
                    "Client {}: configured={}",
                    client.client.display_name(),
                    client.configured
                );
            }
        }
    }
}

fn format_relative_time(epoch_secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let diff = now.saturating_sub(epoch_secs);
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

pub fn print_projects(projects: &[crate::registry::ProjectRecord]) {
    let is_tty = io::stdout().is_terminal();

    if is_tty {
        println!();
        println!(
            "  {ACCENT}agent-mem projects{RESET}  {SUBTLE}·{RESET}  {MUTED}canonical registry ({} registered){RESET}",
            projects.len()
        );
        println!();

        if projects.is_empty() {
            println!(
                "    {SUBTLE}·{RESET}  {MUTED}no projects registered yet  ·  run 'agent-mem init' in any repository{RESET}"
            );
            println!();
            return;
        }

        for p in projects {
            let last_str = format_relative_time(p.last_accessed);
            let git_info = p
                .git_remote
                .as_deref()
                .map(|r| format!("  {SUBTLE}({}){RESET}", r))
                .unwrap_or_default();

            println!(
                "    {EMERALD}▶{RESET}  {ACCENT}{:<20}{RESET}  {MUTED}{:>2} active{RESET}  {SUBTLE}·{RESET}  {MUTED}{:>2} sessions{RESET}  {SUBTLE}·{RESET}  {MUTED}{}{RESET}",
                p.name, p.rules_count, p.sessions_count, last_str
            );
            println!(
                "       {SUBTLE}path:{RESET} {MUTED}{}{}{RESET}",
                p.canonical_path, git_info
            );
        }

        println!();
        println!(
            "  {MUTED}Use 'agent-mem projects prune' to clean up moved or deleted repositories.{RESET}"
        );
        println!();
    } else {
        println!("ID\tNAME\tRULES\tSESSIONS\tPATH\tREMOTE");
        for p in projects {
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                p.id,
                p.name,
                p.rules_count,
                p.sessions_count,
                p.canonical_path,
                p.git_remote.as_deref().unwrap_or("-")
            );
        }
    }
}
