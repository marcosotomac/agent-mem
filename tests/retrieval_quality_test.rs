use agent_mem::store::Store;

struct QueryCase {
    query: &'static str,
    expected_key: &'static str,
}

#[test]
fn retrieval_eval_noisy_daily_queries_preserve_recall_and_context_efficiency() {
    let mut store = Store::open_in_memory().unwrap();
    let relevant = [
        (
            "gotcha/auth/issuer",
            "Validate the JWT issuer and audience before accepting a token to prevent cross-tenant account access",
        ),
        (
            "gotcha/auth/constant-time",
            "Compare HMAC signatures and API tokens in constant time to block timing side channels",
        ),
        (
            "gotcha/db/writer-lock",
            "Start SQLite write transactions with BEGIN IMMEDIATE to prevent SQLITE BUSY lock upgrade deadlocks",
        ),
        (
            "decision/payments/idempotency",
            "Require an Idempotency-Key on every mutating payment request so retries cannot create duplicate charges",
        ),
        (
            "pattern/cache/single-flight",
            "Use a single-flight guard around cache misses to stop a thundering herd and cache stampede",
        ),
        (
            "pattern/queue/outbox",
            "Persist domain events in a transactional outbox and publish after commit to avoid lost messages",
        ),
        (
            "pattern/deploy/expand-contract",
            "Use expand and contract database migrations so mixed application versions remain compatible during rolling deploys",
        ),
        (
            "rule/observability/correlation",
            "Propagate trace and correlation identifiers across HTTP and queue boundaries",
        ),
        (
            "rule/privacy/log-redaction",
            "Redact access tokens passwords and personal data before structured logging",
        ),
        (
            "decision/api/cursor-pagination",
            "Use cursor pagination with a stable tiebreaker instead of offset for mutable datasets",
        ),
    ];
    for (key, value) in relevant {
        store.set(key, value).unwrap();
    }

    // Common vocabulary makes the corpus deliberately ambiguous. Each distractor
    // shares operational terms with the qrels but not their decisive combination.
    for index in 0..600 {
        store
            .set(
                &format!("background/service-{index:04}"),
                &format!(
                    "Service {index} handles HTTP requests, database records, cache entries, tokens, queue messages, retries, logging, and rolling deployments"
                ),
            )
            .unwrap();
    }

    let cases = [
        QueryCase {
            query: "JWT issuer audience cross tenant",
            expected_key: "gotcha/auth/issuer",
        },
        QueryCase {
            query: "stop forged tenant token with issuer validation",
            expected_key: "gotcha/auth/issuer",
        },
        QueryCase {
            query: "constant time HMAC signature comparison",
            expected_key: "gotcha/auth/constant-time",
        },
        QueryCase {
            query: "prevent token timing leak constant comparison",
            expected_key: "gotcha/auth/constant-time",
        },
        QueryCase {
            query: "SQLite BEGIN IMMEDIATE writer deadlocks",
            expected_key: "gotcha/db/writer-lock",
        },
        QueryCase {
            query: "avoid lock upgrade failure with immediate transaction",
            expected_key: "gotcha/db/writer-lock",
        },
        QueryCase {
            query: "duplicate payment retry idempotency key",
            expected_key: "decision/payments/idempotency",
        },
        QueryCase {
            query: "prevent charging twice using payment idempotency",
            expected_key: "decision/payments/idempotency",
        },
        QueryCase {
            query: "single flight cache stampede herd",
            expected_key: "pattern/cache/single-flight",
        },
        QueryCase {
            query: "collapse concurrent cache recomputation single flight",
            expected_key: "pattern/cache/single-flight",
        },
        QueryCase {
            query: "lost event publish transactional outbox commit",
            expected_key: "pattern/queue/outbox",
        },
        QueryCase {
            query: "message disappears after commit use an outbox",
            expected_key: "pattern/queue/outbox",
        },
        QueryCase {
            query: "rolling deploy expand contract database migration",
            expected_key: "pattern/deploy/expand-contract",
        },
        QueryCase {
            query: "database change compatible across old and new versions expand",
            expected_key: "pattern/deploy/expand-contract",
        },
        QueryCase {
            query: "trace correlation identifiers queue HTTP",
            expected_key: "rule/observability/correlation",
        },
        QueryCase {
            query: "follow request across worker logs correlation id",
            expected_key: "rule/observability/correlation",
        },
        QueryCase {
            query: "redact passwords tokens personal data logs",
            expected_key: "rule/privacy/log-redaction",
        },
        QueryCase {
            query: "keep secrets and PII out of logs with token redaction",
            expected_key: "rule/privacy/log-redaction",
        },
        QueryCase {
            query: "cursor pagination stable tiebreaker mutable dataset",
            expected_key: "decision/api/cursor-pagination",
        },
        QueryCase {
            query: "avoid duplicate rows while paging changing data with cursor",
            expected_key: "decision/api/cursor-pagination",
        },
    ];

    let full_bytes: usize = store
        .dump_all()
        .unwrap()
        .iter()
        .map(|rule| rule.key.len() + rule.val.len())
        .sum();
    let mut hits_at_five = 0usize;
    let mut reciprocal_rank = 0.0;
    let mut returned_bytes = 0usize;
    for case in &cases {
        let results = store.find(case.query).unwrap();
        let rank = results
            .iter()
            .position(|rule| rule.key == case.expected_key)
            .map(|index| index + 1);
        if rank.is_some_and(|value| value <= 5) {
            hits_at_five += 1;
        }
        if let Some(rank) = rank {
            reciprocal_rank += 1.0 / rank as f64;
        }
        returned_bytes += results
            .iter()
            .take(5)
            .map(|rule| rule.key.len() + rule.val.len())
            .sum::<usize>();
    }

    let recall_at_five = hits_at_five as f64 / cases.len() as f64;
    let mrr = reciprocal_rank / cases.len() as f64;
    let mean_payload_ratio = returned_bytes as f64 / (full_bytes * cases.len()) as f64;
    println!(
        "retrieval eval: cases={} recall@5={recall_at_five:.3} mrr={mrr:.3} mean_top5_payload={:.3}%",
        cases.len(),
        mean_payload_ratio * 100.0
    );
    assert_eq!(recall_at_five, 1.0, "all qrels must appear in the top five");
    assert_eq!(
        mrr, 1.0,
        "all fixed qrels should rank first, got MRR {mrr:.3}"
    );
    assert!(
        mean_payload_ratio <= 0.02,
        "top-five retrieval should emit <=2% of full-corpus bytes, got {:.2}%",
        mean_payload_ratio * 100.0
    );
}
