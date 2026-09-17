use agent_mem::init::init_project;
use agent_mem::mcp::{JsonRpcRequest, McpServer};
use agent_mem::{BatchRule, Store};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "agent_mem_real_{label}_{}_{}_{}",
            std::process::id(),
            nonce,
            counter
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run git {args:?}: {error}"));
    assert!(
        output.status.success(),
        "git {args:?} failed in {}\nstdout: {}\nstderr: {}",
        cwd.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn configure_git(repo: &Path, identity: &str) {
    run_git(repo, &["config", "user.name", identity]);
    run_git(
        repo,
        &[
            "config",
            "user.email",
            &format!("{}@example.invalid", identity.to_ascii_lowercase()),
        ],
    );
}

fn call_tool(server: &McpServer, id: u64, name: &str, arguments: serde_json::Value) -> String {
    let response = server
        .handle_request(&JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(id)),
            method: "tools/call".into(),
            params: json!({"name": name, "arguments": arguments}),
        })
        .expect("MCP response")
        .result
        .expect("MCP result");
    assert_ne!(
        response.get("isError").and_then(|v| v.as_bool()),
        Some(true)
    );
    response["content"][0]["text"]
        .as_str()
        .expect("text result")
        .to_string()
}

#[test]
fn cloned_teammates_merge_conflicting_and_lossless_rules_without_data_loss() {
    let sandbox = TestDir::new("team_clone");
    let origin = sandbox.path().join("origin");
    let alice = sandbox.path().join("alice");
    fs::create_dir_all(&origin).unwrap();

    run_git(&origin, &["init", "-b", "main"]);
    configure_git(&origin, "Integrator");
    init_project(&origin).unwrap();

    let origin_db = origin.join(".agent-mem/mem.db");
    let origin_rules = origin.join(".agent-rules");
    {
        let mut store = Store::open(&origin_db, true).unwrap();
        store
            .set_entry(
                "architecture/core",
                "Keep domain logic independent from transport",
                Some("src/domain/mod.rs:1"),
                Some("decision"),
            )
            .unwrap();
        store
            .set("team/auth-mode", "Use RS256 while migration is pending")
            .unwrap();
        store.sync_export(&origin_rules).unwrap();
    }
    run_git(&origin, &["add", "."]);
    run_git(&origin, &["commit", "-m", "chore: seed shared memory"]);

    run_git(
        sandbox.path(),
        &["clone", origin.to_str().unwrap(), alice.to_str().unwrap()],
    );
    configure_git(&alice, "Alice");
    init_project(&alice).unwrap();
    let alice_db = alice.join(".agent-mem/mem.db");
    let alice_rules = alice.join(".agent-rules");
    {
        let store = Store::open(&alice_db, false).unwrap();
        assert_eq!(
            store.dump_all().unwrap().len(),
            2,
            "clone init must import team rules"
        );
    }
    run_git(&alice, &["checkout", "-b", "alice/auth-hardening"]);
    {
        let mut store = Store::open(&alice_db, true).unwrap();
        store
            .set("team/auth-mode", "Alice: migrate to Ed25519")
            .unwrap();
        store
            .set_entry(
                "gotcha/auth/clock-skew",
                "Accept five seconds of skew\nNever disable issuer checks @ staging",
                Some("services/auth/src/jwt.rs:42"),
                Some("gotcha"),
            )
            .unwrap();
        store
            .relate("team/auth-mode", "mitigates", "gotcha/auth/clock-skew")
            .unwrap();
        store.sync_export(&alice_rules).unwrap();
    }
    run_git(&alice, &["add", ".agent-rules"]);
    run_git(
        &alice,
        &["commit", "-m", "fix(auth): harden token validation"],
    );
    run_git(&alice, &["push", "origin", "alice/auth-hardening"]);

    {
        let mut store = Store::open(&origin_db, true).unwrap();
        store
            .set("team/auth-mode", "Integrator: migrate to ES384")
            .unwrap();
        store
            .set_entry(
                "decision/api/idempotency",
                "Require idempotency keys for payment mutations",
                Some("services/api/src/payments.rs:18"),
                Some("decision"),
            )
            .unwrap();
        store.sync_export(&origin_rules).unwrap();
    }
    run_git(&origin, &["add", ".agent-rules"]);
    run_git(
        &origin,
        &["commit", "-m", "feat(api): document idempotency"],
    );
    run_git(
        &origin,
        &[
            "merge",
            "alice/auth-hardening",
            "-m",
            "merge: auth hardening",
        ],
    );

    let mut merged = Store::open(&origin_db, true).unwrap();
    let report = merged.sync_with_file(&origin_rules).unwrap();
    assert_eq!(report.conflicts_resolved, 1);
    assert_eq!(report.total, 4);
    assert!(
        matches!(
            merged.get("team/auth-mode").unwrap().as_deref(),
            Some("Alice: migrate to Ed25519" | "Integrator: migrate to ES384")
        ),
        "same-key branch conflict must resolve to one complete value"
    );
    assert_eq!(
        merged.get("gotcha/auth/clock-skew").unwrap().as_deref(),
        Some("Accept five seconds of skew\nNever disable issuer checks @ staging")
    );
    assert_eq!(merged.get_relations("team/auth-mode").unwrap().len(), 1);
    assert!(merged.session_list(20).unwrap().iter().any(|(_, summary)| {
        summary.contains("[conflict-resolved]") && summary.contains("team/auth-mode")
    }));

    merged.sync_export(&origin_rules).unwrap();
    let clean = fs::read_to_string(&origin_rules).unwrap();
    assert_eq!(clean.matches("team/auth-mode").count(), 2); // rule plus relation
    assert!(
        clean.contains("[rule-json-v1]"),
        "multiline value must use lossless encoding"
    );

    let replica_path = sandbox.path().join("replica.db");
    let mut replica = Store::open(&replica_path, true).unwrap();
    replica.sync_with_file(&origin_rules).unwrap();
    assert_eq!(replica.export_rules_text().unwrap(), clean);
    assert_eq!(
        replica.dump_all().unwrap().len(),
        merged.dump_all().unwrap().len()
    );
}

#[test]
fn corruption_and_conflict_storm_are_atomic_bounded_and_auditable() {
    let sandbox = TestDir::new("conflict_storm");
    let db = sandbox.path().join("mem.db");
    let rules = sandbox.path().join(".agent-rules");
    let mut store = Store::open(&db, true).unwrap();
    store
        .set("preserved/core", "must survive failed sync")
        .unwrap();
    store.set("preserved/target", "relation endpoint").unwrap();
    store
        .relate("preserved/core", "depends_on", "preserved/target")
        .unwrap();
    store.session_add("before corruption").unwrap();
    let before = store.export_rules_text().unwrap();

    fs::write(
        &rules,
        "new/rule = must not partially import\n[rule-json-v1] {broken-json\n",
    )
    .unwrap();
    assert!(store.sync_with_file(&rules).is_err());
    assert_eq!(store.export_rules_text().unwrap(), before);
    assert_eq!(store.get_all_relations().unwrap().len(), 1);
    assert_eq!(store.session_list(100).unwrap().len(), 1);

    let mut storm = String::from("stable/rule = stable value\n");
    for version in 0..40 {
        storm.push_str(&format!(
            "gotcha/auth = branch-{version}: validate issuer and audience\n"
        ));
    }
    storm.push_str("[rel] gotcha/auth -> mitigates -> stable/rule\n");
    fs::write(&rules, &storm).unwrap();

    let report = store.sync_with_file(&rules).unwrap();
    assert_eq!(report.conflicts_resolved, 39);
    assert_eq!(report.total, 2);
    assert_eq!(
        store.get("gotcha/auth").unwrap().as_deref(),
        Some("branch-39: validate issuer and audience")
    );
    assert_eq!(store.get_relations("gotcha/auth").unwrap().len(), 1);
    assert!(
        store
            .find("branch-39 audience")
            .unwrap()
            .iter()
            .any(|r| r.key == "gotcha/auth")
    );
    assert!(
        store.find("branch-0").unwrap().is_empty(),
        "FTS must not retain losing values"
    );

    let sessions = store.session_list(100).unwrap();
    assert!(
        sessions.len() <= 20,
        "conflict provenance must respect the session ring buffer"
    );
    assert!(
        sessions
            .iter()
            .all(|(_, summary)| summary.contains("[conflict-resolved]"))
    );

    let session_count = sessions.len();
    let noop = store.sync_with_file(&rules).unwrap();
    assert_eq!(noop.conflicts_resolved, 0);
    assert_eq!(store.session_list(100).unwrap().len(), session_count);
}

#[test]
fn monorepo_context_prioritizes_exact_paths_topics_and_graph_neighbors() {
    let sandbox = TestDir::new("monorepo_context");
    let db = sandbox.path().join("mem.db");
    let mut store = Store::open(&db, true).unwrap();

    let fixtures: Vec<(String, String, String)> = (0..600)
        .map(|i| {
            (
                format!("service/{i:03}/convention"),
                format!("Routine convention for background service {i}"),
                format!("services/worker_{i:03}/src/mod.rs:10"),
            )
        })
        .collect();
    let batch: Vec<BatchRule<'_>> = fixtures
        .iter()
        .map(|(key, val, anchor)| BatchRule {
            key,
            val,
            anchor: Some(anchor),
            kind: Some("rule"),
            relation: None,
        })
        .collect();
    store.set_batch(&batch).unwrap();
    store
        .set_entry(
            "decision/api/request-validation",
            "Validate authorization before parsing payment mutations",
            Some("services/api/src/mod.rs:12"),
            Some("decision"),
        )
        .unwrap();
    store
        .set_entry(
            "gotcha/api/body-limit",
            "Reject request bodies above the configured limit",
            Some("services/api/src/http/body.rs:8"),
            Some("gotcha"),
        )
        .unwrap();
    store
        .set_entry(
            "decision/database/transactions",
            "Use transactions for payment state changes",
            Some("services/database/src/mod.rs:4"),
            Some("decision"),
        )
        .unwrap();
    store
        .set(
            "global/error-format",
            "Return stable machine-readable error codes",
        )
        .unwrap();
    store
        .relate(
            "decision/api/request-validation",
            "mitigates",
            "gotcha/api/body-limit",
        )
        .unwrap();

    let started = Instant::now();
    for _ in 0..100 {
        let (rules, _, _) = store
            .context_for_files(&["services/api/src/mod.rs".into()], None, 4)
            .unwrap();
        assert_eq!(rules[0].key, "decision/api/request-validation");
        assert!(rules.iter().any(|r| r.key == "gotcha/api/body-limit"));
        assert!(rules.iter().all(|r| !r.key.starts_with("service/")));
        assert!(
            rules
                .iter()
                .all(|r| r.key != "decision/database/transactions")
        );
    }
    let elapsed = started.elapsed();

    let (topic_rules, _, _) = store
        .context_filtered(Some("services/api/src/mod.rs"), Some("decision/api"), 10)
        .unwrap();
    assert_eq!(topic_rules.len(), 1);
    assert_eq!(topic_rules[0].key, "decision/api/request-validation");

    assert!(
        elapsed < Duration::from_secs(2),
        "100 context lookups over 604 rules took {elapsed:?}"
    );
    eprintln!("real-world monorepo context: 100 exact-path lookups over 604 rules in {elapsed:?}");
}

#[test]
fn mcp_context_budget_preserves_critical_and_global_rules_under_large_payloads() {
    let sandbox = TestDir::new("mcp_budget");
    let project_db = sandbox.path().join(".agent-mem/mem.db");
    let global_db = sandbox.path().join("global.db");
    let mut project = Store::open(&project_db, true).unwrap();

    let large_value = format!(
        "Production incident trace with repeated frames: {}",
        "frame::retry -> frame::timeout; ".repeat(1500)
    );
    for i in 0..24 {
        project
            .set(&format!("incident/{i:02}"), &large_value)
            .unwrap();
    }
    project
        .set_entry(
            "decision/payments/authorization",
            "Authorize the tenant before loading or mutating a payment",
            Some("services/payments/src/handler.rs:17"),
            Some("decision"),
        )
        .unwrap();
    project
        .set_entry(
            "gotcha/payments/replay",
            "Reject a reused idempotency key with a different payload",
            Some("services/payments/src/idempotency.rs:31"),
            Some("gotcha"),
        )
        .unwrap();
    project
        .relate(
            "decision/payments/authorization",
            "relates_to",
            "gotcha/payments/replay",
        )
        .unwrap();
    project
        .session_add("Investigating payment timeout incident")
        .unwrap();
    let full_payload_bytes: usize = project
        .dump_all()
        .unwrap()
        .iter()
        .map(|rule| rule.key.len() + rule.val.len())
        .sum();
    drop(project);

    let mut global = Store::open(&global_db, true).unwrap();
    global
        .set(
            "personal/security",
            "Never weaken authorization to make a test pass",
        )
        .unwrap();
    global
        .set(
            "personal/style",
            "Prefer explicit errors over silent fallbacks",
        )
        .unwrap();
    drop(global);

    let server = McpServer::with_paths(sandbox.path().to_path_buf(), global_db);
    let text = call_tool(
        &server,
        1,
        "mem_context",
        json!({
            "scope": "all",
            "files": "services/payments/src/handler.rs",
            "limit": 12
        }),
    );

    assert!(text.len() <= 16 * 1024);
    assert!(text.contains("decision/payments/authorization"));
    assert!(text.contains("gotcha/payments/replay"));
    assert!(text.contains("== GLOBAL PREFERENCES =="));
    assert!(text.contains("personal/security"));
    assert!(text.contains("personal/style"));
    assert!(text.contains("== SESSIONS =="));
    assert!(!text.ends_with("...[truncated: output budget]"));
    assert!(
        text.len() * 20 < full_payload_bytes,
        "selected context should reduce this fixture by more than 95%"
    );
}

#[test]
fn concurrent_writers_readers_and_exports_converge_to_a_lossless_replica() {
    let sandbox = TestDir::new("concurrency");
    let db = Arc::new(sandbox.path().join("mem.db"));
    let rules = Arc::new(sandbox.path().join(".agent-rules"));
    {
        let mut store = Store::open(&db, true).unwrap();
        for i in 0..20 {
            store.set(&format!("base/{i}"), "shared baseline").unwrap();
        }
        store.sync_export(&rules).unwrap();
    }

    let barrier = Arc::new(Barrier::new(7));
    let mut handles = Vec::new();
    for worker in 0..4 {
        let db = Arc::clone(&db);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let mut store = Store::open(&db, true).unwrap();
            let owned: Vec<(String, String, String)> = (0..100)
                .map(|i| {
                    (
                        format!("worker/{worker}/rule/{i:03}"),
                        format!("worker {worker} preserves event {i}"),
                        format!("services/worker_{worker}/src/job.rs:{i}"),
                    )
                })
                .collect();
            let batch: Vec<BatchRule<'_>> = owned
                .iter()
                .map(|(key, val, anchor)| BatchRule {
                    key,
                    val,
                    anchor: Some(anchor),
                    kind: Some("rule"),
                    relation: Some(("depends_on", "base/0")),
                })
                .collect();
            barrier.wait();
            store.set_batch(&batch).unwrap();
        }));
    }
    for _ in 0..2 {
        let db = Arc::clone(&db);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let store = Store::open(&db, false).unwrap();
            barrier.wait();
            for _ in 0..50 {
                let _ = store.find("preserves event").unwrap();
                let _ = store
                    .context_for_files(&["services/worker_2/src/job.rs".into()], None, 10)
                    .unwrap();
                thread::yield_now();
            }
        }));
    }
    {
        let db = Arc::clone(&db);
        let rules = Arc::clone(&rules);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let store = Store::open(&db, false).unwrap();
            barrier.wait();
            for _ in 0..30 {
                store.export_to_file(&rules).unwrap();
                let snapshot = fs::read_to_string(&*rules).unwrap();
                assert!(snapshot.starts_with("# .agent-rules"));
                thread::yield_now();
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let store = Store::open(&db, false).unwrap();
    assert_eq!(store.dump_all().unwrap().len(), 420);
    assert_eq!(store.get_all_relations().unwrap().len(), 400);
    store.sync_export(&rules).unwrap();

    let replica_db = sandbox.path().join("replica.db");
    let mut replica = Store::open(&replica_db, true).unwrap();
    replica.sync_with_file(&rules).unwrap();
    assert_eq!(replica.dump_all().unwrap().len(), 420);
    assert_eq!(replica.get_all_relations().unwrap().len(), 400);
    assert_eq!(
        replica.export_rules_text().unwrap(),
        store.export_rules_text().unwrap()
    );

    let temp_artifacts = fs::read_dir(sandbox.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
        .count();
    assert_eq!(
        temp_artifacts, 0,
        "atomic exports must clean temporary files"
    );
}

#[test]
fn batch_and_noop_paths_are_measurably_cheaper_than_repeated_work() {
    let sandbox = TestDir::new("performance");
    let individual_db = sandbox.path().join("individual.db");
    let batch_db = sandbox.path().join("batch.db");
    let rules = sandbox.path().join(".agent-rules");
    let fixtures: Vec<(String, String, String)> = (0..500)
        .map(|i| {
            (
                format!("perf/rule/{i:04}"),
                format!("Production convention {i} for authorization and retries"),
                format!("services/perf/src/module_{:03}.rs:{}", i % 50, i + 1),
            )
        })
        .collect();

    let mut individual = Store::open(&individual_db, true).unwrap();
    let single_started = Instant::now();
    for (key, val, anchor) in &fixtures {
        individual
            .set_entry(key, val, Some(anchor), Some("rule"))
            .unwrap();
    }
    let single_elapsed = single_started.elapsed();

    let mut batched = Store::open(&batch_db, true).unwrap();
    let batch_rules: Vec<BatchRule<'_>> = fixtures
        .iter()
        .map(|(key, val, anchor)| BatchRule {
            key,
            val,
            anchor: Some(anchor),
            kind: Some("rule"),
            relation: None,
        })
        .collect();
    let batch_started = Instant::now();
    batched.set_batch(&batch_rules).unwrap();
    let batch_elapsed = batch_started.elapsed();

    assert_eq!(batched.dump_all().unwrap().len(), 500);
    assert_eq!(individual.dump_all().unwrap().len(), 500);
    assert!(
        batch_elapsed < single_elapsed,
        "one transaction ({batch_elapsed:?}) must beat 500 transactions ({single_elapsed:?})"
    );

    batched.sync_export(&rules).unwrap();
    let replica_db = sandbox.path().join("sync-replica.db");
    let mut replica = Store::open(&replica_db, true).unwrap();
    let full_started = Instant::now();
    replica.sync_with_file(&rules).unwrap();
    let full_elapsed = full_started.elapsed();
    let noop_started = Instant::now();
    for _ in 0..20 {
        let report = replica.sync_with_file(&rules).unwrap();
        assert_eq!(report.total, 500);
        assert_eq!(report.conflicts_resolved, 0);
    }
    let average_noop = noop_started.elapsed() / 20;
    assert!(
        average_noop < full_elapsed,
        "no-op sync average ({average_noop:?}) must beat full sync ({full_elapsed:?})"
    );

    eprintln!(
        "real-world performance: 500 individual writes={single_elapsed:?}, batch={batch_elapsed:?}, full sync={full_elapsed:?}, no-op avg={average_noop:?}"
    );
}
