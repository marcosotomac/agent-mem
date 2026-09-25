#![cfg(feature = "semantic-local")]

use agent_mem::store::Store;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

struct SemanticCase {
    query: &'static str,
    expected_key: &'static str,
}

#[test]
#[ignore = "downloads the local embedding model and measures real inference"]
fn semantic_fallback_recovers_paraphrases_and_refreshes_changed_memories() {
    let dir = std::env::temp_dir().join(format!(
        "agent_mem_semantic_eval_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("mem.db");
    let mut store = Store::open(&db_path, true).unwrap();

    let relevant = [
        (
            "decision/payment-write-safety",
            "Require an Idempotency-Key on every mutating payment request so retries cannot create duplicate charges",
        ),
        (
            "gotcha/signature-check",
            "Compare HMAC signatures and API tokens in constant time to block timing side channels",
        ),
        (
            "pattern/event-delivery",
            "Persist domain events in a transactional outbox and publish only after the database commit",
        ),
        (
            "decision/list-navigation",
            "Use cursor pagination with a stable tiebreaker instead of offsets for mutable datasets",
        ),
        (
            "rule/sensitive-telemetry",
            "Redact access tokens passwords and personally identifiable information before structured logging",
        ),
    ];
    for (key, value) in relevant {
        store.set(key, value).unwrap();
    }
    for index in 0..200 {
        store
            .set(
                &format!("background/service-{index:04}"),
                &format!(
                    "Service {index} processes routine HTTP traffic, cache refreshes, database records, queue workers, dashboards, and deployment checks"
                ),
            )
            .unwrap();
    }

    let report = store.semantic_rebuild().unwrap();
    assert_eq!(report.records_count, 205);
    assert!(report.index_bytes > 0);
    let status = store.semantic_status().unwrap();
    assert!(status.enabled);
    assert!(!status.dirty);
    let first_index = status.index_path;

    let cases = [
        SemanticCase {
            query: "evitar que una persona pague dos veces por reintentar",
            expected_key: "decision/payment-write-safety",
        },
        SemanticCase {
            query: "impedir que descubran secretos midiendo cuánto demora la comparación",
            expected_key: "gotcha/signature-check",
        },
        SemanticCase {
            query: "que los avisos no desaparezcan si el proceso cae después de guardar datos",
            expected_key: "pattern/event-delivery",
        },
        SemanticCase {
            query: "recorrer una lista cambiante sin saltar ni repetir filas",
            expected_key: "decision/list-navigation",
        },
        SemanticCase {
            query: "ocultar credenciales y datos personales en registros de diagnóstico",
            expected_key: "rule/sensitive-telemetry",
        },
    ];

    let started = Instant::now();
    let mut hits_at_five = 0usize;
    let mut reciprocal_rank = 0.0;
    for case in &cases {
        let results = store.find(case.query).unwrap();
        println!(
            "query={:?} results={:?}",
            case.query,
            results
                .iter()
                .map(|rule| rule.key.as_str())
                .collect::<Vec<_>>()
        );
        let rank = results
            .iter()
            .position(|rule| rule.key == case.expected_key)
            .map(|index| index + 1);
        if rank.is_some_and(|rank| rank <= 5) {
            hits_at_five += 1;
        }
        if let Some(rank) = rank {
            reciprocal_rank += 1.0 / rank as f64;
        }
    }
    let recall_at_five = hits_at_five as f64 / cases.len() as f64;
    let mrr = reciprocal_rank / cases.len() as f64;
    println!(
        "semantic eval: build_ms={} index_bytes={} recall@5={recall_at_five:.3} mrr={mrr:.3} query_total_ms={}",
        report.elapsed_ms,
        report.index_bytes,
        started.elapsed().as_millis()
    );
    assert!(
        recall_at_five >= 0.8,
        "semantic Recall@5 must be >= 0.8, got {recall_at_five:.3}"
    );
    assert!(mrr >= 0.7, "semantic MRR must be >= 0.7, got {mrr:.3}");

    store
        .set(
            "decision/payment-write-safety",
            "Render the account preferences page with compact spacing",
        )
        .unwrap();
    store
        .set(
            "decision/incident-rotation",
            "Rotate credentials after incident containment to prevent reuse of compromised keys",
        )
        .unwrap();
    assert!(store.semantic_status().unwrap().dirty);
    assert!(!store.find("incident rotation").unwrap().is_empty());
    assert!(
        store.semantic_status().unwrap().dirty,
        "strong lexical search should not pay the embedding refresh cost"
    );
    let refresh_started = Instant::now();
    let refreshed_results = store
        .find("evitar que una persona pague dos veces por reintentar")
        .unwrap();
    println!(
        "semantic auto refresh after two changed memories: {} ms",
        refresh_started.elapsed().as_millis()
    );
    assert!(
        refreshed_results
            .iter()
            .all(|rule| rule.key != "decision/payment-write-safety"
                || rule.val == "Render the account preferences page with compact spacing"),
        "search must never return an old memory value"
    );
    let status = store.semantic_status().unwrap();
    assert!(
        !status.dirty,
        "semantic search should refresh a dirty index"
    );
    assert_eq!(status.records_count, 206);
    assert_ne!(status.index_path, first_index);
    let new_hits = store
        .find("rotar credenciales después de contener un incidente")
        .unwrap();
    assert!(
        new_hits
            .iter()
            .take(5)
            .any(|rule| rule.key == "decision/incident-rotation"),
        "new memories must be available to semantic search without a manual build"
    );

    store
        .set(
            "decision/incident-rotation",
            "Rotate credentials after incident containment and invalidate compromised keys",
        )
        .unwrap();
    let searches: Vec<_> = (0..2)
        .map(|_| {
            let path = db_path.clone();
            std::thread::spawn(move || {
                Store::open(&path, false)
                    .unwrap()
                    .find("evitar que una persona pague dos veces por reintentar")
                    .unwrap()
            })
        })
        .collect();
    for search in searches {
        assert!(!search.join().unwrap().is_empty());
    }
    assert!(!store.semantic_status().unwrap().dirty);

    assert!(store.semantic_clear().unwrap() >= 1);
    assert!(!store.semantic_status().unwrap().enabled);
    store
        .find("rotar credenciales después de contener un incidente")
        .unwrap();
    assert!(
        !store.semantic_status().unwrap().enabled,
        "explicitly disabling semantic search must remain effective"
    );
    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}
