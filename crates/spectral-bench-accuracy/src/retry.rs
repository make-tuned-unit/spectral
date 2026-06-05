//! Retry-with-backoff for transient API failures.
//!
//! Classifies errors into transport (retry), transient HTTP 429/5xx (retry),
//! and auth 4xx (fail fast). Exponential backoff with jitter.

use anyhow::Result;
use std::time::Duration;

/// Maximum total retry time per question (60 seconds).
const MAX_TOTAL_RETRY_MS: u64 = 60_000;

/// Outcome of a retried call.
pub enum CallOutcome<T> {
    /// Succeeded (possibly after retries).
    Success { value: T, retry_count: u32 },
    /// Exhausted retries on a transient/transport error.
    TransportFailure { error: String, retry_count: u32 },
    /// Hit a non-retryable auth/validation error (401/403/400). No retries attempted.
    AuthFailure { error: String },
}

/// Classify an error message to decide retry behavior.
enum ErrorClass {
    /// Connection/transport error — retry.
    Transport,
    /// HTTP 429 or 5xx — retry.
    TransientHttp,
    /// HTTP 401/403/400 — fail fast.
    Auth,
}

fn classify_error(err_msg: &str) -> ErrorClass {
    // Auth / validation errors — fail fast
    if err_msg.contains("returned 401")
        || err_msg.contains("returned 403")
        || err_msg.contains("returned 400")
    {
        return ErrorClass::Auth;
    }
    // Transient HTTP errors — retry
    if err_msg.contains("returned 429") || err_msg.contains("returned 529") {
        return ErrorClass::TransientHttp;
    }
    // Check for any 5xx
    if let Some(pos) = err_msg.find("returned 5") {
        if err_msg[pos..].len() >= "returned 5xx".len() {
            return ErrorClass::TransientHttp;
        }
    }
    // Everything else (connection errors, timeouts, etc.) — treat as transport
    ErrorClass::Transport
}

/// Execute `f` with retry-on-transient-failure.
///
/// - `max_attempts`: total attempts including the first (default 4 = 1 + 3 retries).
/// - `question_id`: for logging.
/// - `caller`: "actor" or "judge", for logging.
pub fn with_retry<F, T>(max_attempts: u32, question_id: &str, caller: &str, f: F) -> CallOutcome<T>
where
    F: Fn() -> Result<T>,
{
    let mut retry_count: u32 = 0;
    let mut total_delay_ms: u64 = 0;

    for attempt in 1..=max_attempts {
        match f() {
            Ok(value) => {
                return CallOutcome::Success { value, retry_count };
            }
            Err(e) => {
                let err_msg = format!("{e}");
                match classify_error(&err_msg) {
                    ErrorClass::Auth => {
                        eprintln!(
                            "  [{caller}] {question_id}: auth failure (attempt {attempt}): {err_msg}"
                        );
                        return CallOutcome::AuthFailure { error: err_msg };
                    }
                    ErrorClass::Transport | ErrorClass::TransientHttp => {
                        if attempt >= max_attempts || total_delay_ms >= MAX_TOTAL_RETRY_MS {
                            eprintln!(
                                "  [{caller}] {question_id}: exhausted {attempt} attempts: {err_msg}"
                            );
                            return CallOutcome::TransportFailure {
                                error: err_msg,
                                retry_count,
                            };
                        }
                        // Exponential backoff: 1s, 2s, 4s, ... capped at 16s
                        let base_ms = 1000u64 * 2u64.pow(retry_count);
                        let base_ms = base_ms.min(16_000);
                        // Add jitter: ±25%
                        let jitter = (base_ms / 4) as i64;
                        let jitter_val = (std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .subsec_nanos() as i64)
                            % (jitter * 2 + 1)
                            - jitter;
                        let delay_ms = (base_ms as i64 + jitter_val).max(100) as u64;
                        let delay_ms = delay_ms.min(MAX_TOTAL_RETRY_MS - total_delay_ms);

                        retry_count += 1;
                        total_delay_ms += delay_ms;

                        eprintln!(
                            "  [{caller}] {question_id}: retry {retry_count} in {delay_ms}ms \
                             (attempt {attempt}/{max_attempts}): {err_msg}"
                        );
                        std::thread::sleep(Duration::from_millis(delay_ms));
                    }
                }
            }
        }
    }

    unreachable!("loop should have returned")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn succeeds_on_first_try() {
        let outcome = with_retry(4, "q1", "test", || Ok(42));
        match outcome {
            CallOutcome::Success { value, retry_count } => {
                assert_eq!(value, 42);
                assert_eq!(retry_count, 0);
            }
            _ => panic!("expected success"),
        }
    }

    #[test]
    fn retries_on_transport_error_then_succeeds() {
        let attempt = Cell::new(0u32);
        let outcome = with_retry(4, "q-transport", "test", || {
            let n = attempt.get() + 1;
            attempt.set(n);
            if n < 3 {
                Err(anyhow::anyhow!("error sending request for url"))
            } else {
                Ok("recovered")
            }
        });
        match outcome {
            CallOutcome::Success { value, retry_count } => {
                assert_eq!(value, "recovered");
                assert_eq!(retry_count, 2);
            }
            _ => panic!("expected success after retries"),
        }
    }

    #[test]
    fn retries_on_429_then_succeeds() {
        let attempt = Cell::new(0u32);
        let outcome = with_retry(4, "q-429", "test", || {
            let n = attempt.get() + 1;
            attempt.set(n);
            if n == 1 {
                Err(anyhow::anyhow!("API returned 429: rate limited"))
            } else {
                Ok("ok")
            }
        });
        match outcome {
            CallOutcome::Success { value, retry_count } => {
                assert_eq!(value, "ok");
                assert_eq!(retry_count, 1);
            }
            _ => panic!("expected success after 429 retry"),
        }
    }

    #[test]
    fn retries_on_503_then_succeeds() {
        let attempt = Cell::new(0u32);
        let outcome = with_retry(4, "q-503", "test", || {
            let n = attempt.get() + 1;
            attempt.set(n);
            if n == 1 {
                Err(anyhow::anyhow!("API returned 529: overloaded"))
            } else {
                Ok(99)
            }
        });
        match outcome {
            CallOutcome::Success { value, retry_count } => {
                assert_eq!(value, 99);
                assert_eq!(retry_count, 1);
            }
            _ => panic!("expected success after 5xx retry"),
        }
    }

    #[test]
    fn auth_failure_does_not_retry() {
        let attempt = Cell::new(0u32);
        let outcome: CallOutcome<i32> = with_retry(4, "q-auth", "test", || {
            attempt.set(attempt.get() + 1);
            Err(anyhow::anyhow!(
                "API returned 401 Unauthorized: invalid x-api-key"
            ))
        });
        match outcome {
            CallOutcome::AuthFailure { error } => {
                assert!(error.contains("401"));
                assert_eq!(attempt.get(), 1, "should NOT have retried on 401");
            }
            _ => panic!("expected auth failure"),
        }
    }

    #[test]
    fn persistent_transport_error_exhausts_retries() {
        let attempt = Cell::new(0u32);
        let outcome: CallOutcome<i32> = with_retry(3, "q-exhaust", "test", || {
            attempt.set(attempt.get() + 1);
            Err(anyhow::anyhow!("error sending request for url"))
        });
        match outcome {
            CallOutcome::TransportFailure { error, retry_count } => {
                assert!(error.contains("error sending request"));
                assert_eq!(retry_count, 2); // 3 attempts = 2 retries
                assert_eq!(attempt.get(), 3);
            }
            _ => panic!("expected transport failure after exhaustion"),
        }
    }
}
