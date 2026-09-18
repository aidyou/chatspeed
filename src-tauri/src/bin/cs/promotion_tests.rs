use super::*;
use serde_json::json;

fn projection(state: &str) -> Value {
    json!({ "promotion_id": "promo-0123456789abcdef0123456789abcdef", "state": state })
}

/// The wait budget must cover the widest canary the server can legally run:
/// MAX_CANARY_STAGES stages, each up to MAX_CANARY_TIMEOUT_MS, for two arms,
/// plus supervisor overhead. The previous fixed 900s budget would give up on
/// a legal promotion long before the backend finished.
#[test]
fn the_wait_budget_covers_the_full_canary_ceiling() {
    let floor = minimum_wait_budget_secs();
    let ceiling_secs = types::MAX_CANARY_STAGES as u64 * (types::MAX_CANARY_TIMEOUT_MS / 1000) * 2;
    assert!(
        floor > ceiling_secs,
        "budget {floor}s must exceed the {ceiling_secs}s canary ceiling"
    );
    assert!(
        floor > 900,
        "the legacy 900s budget is known to be too short"
    );
    // A caller override can only raise the budget, never shorten it.
    assert_eq!(resolve_wait_budget_secs(None), floor);
    assert_eq!(resolve_wait_budget_secs(Some(1)), floor);
    assert_eq!(resolve_wait_budget_secs(Some(floor + 1_000)), floor + 1_000);
}

/// A terminal projection is returned immediately without any further fetch.
#[tokio::test]
async fn the_poller_returns_a_terminal_state_without_polling() {
    let fetches = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = fetches.clone();
    let projection = wait_for_terminal(
        projection("promoted"),
        move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(projection("promoted"))
            }
        },
        0,
        1,
    )
    .await
    .expect("terminal");
    assert_eq!(projection["state"], "promoted");
    assert_eq!(
        fetches.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no fetch is needed for an already-terminal reply"
    );
}

/// A non-terminal projection is polled until it terminates.
#[tokio::test]
async fn the_poller_waits_for_a_later_terminal_state() {
    let fetches = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = fetches.clone();
    let projection = wait_for_terminal(
        projection("canary_running"),
        move || {
            let counter = counter.clone();
            async move {
                let seen = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if seen < 2 {
                    Ok(projection("canary_running"))
                } else {
                    Ok(projection("promoted"))
                }
            }
        },
        60,
        1,
    )
    .await
    .expect("terminal");
    assert_eq!(projection["state"], "promoted");
    assert_eq!(fetches.load(std::sync::atomic::Ordering::SeqCst), 2);
}

/// An exhausted budget is a timeout that still names the promotion, so the
/// operator can query it later; it never fabricates a terminal state.
#[tokio::test]
async fn the_poller_times_out_without_a_terminal_state() {
    let error = wait_for_terminal(
        projection("canary_running"),
        || async { Ok(projection("canary_running")) },
        0,
        1,
    )
    .await
    .expect_err("budget exhausted");
    assert!(error.to_string().contains("did not reach a terminal state"));
    assert!(error
        .to_string()
        .contains("promo-0123456789abcdef0123456789abcdef"));
}
