//! Retry policy execution for `http_forward`.
//!
//! The retry loop applies endpoint-configured rules for retryable
//! failures, full-jitter exponential backoff, 429 `Retry-After`
//! handling, and an overall wall-time deadline.

use crate::{
    client::execute_one_attempt,
    config::PreparedConfig,
    error::{Error, Result},
    request::HttpForwardRequest,
    response::HttpForwardResponse,
};
use mechanics_config::EndpointRetryPolicy;
use philharmonic_connector_impl_api::ConnectorCallContext;
use rand::Rng;
use std::time::{Duration, Instant, SystemTime};

pub(crate) async fn execute_with_retry(
    client: &reqwest::Client,
    prepared: &PreparedConfig,
    request: &HttpForwardRequest,
    _ctx: &ConnectorCallContext,
) -> Result<HttpForwardResponse> {
    let policy = prepared.endpoint.retry_policy();
    let max_attempts = policy.max_attempts.max(1);
    let start = Instant::now();
    let deadline = start
        .checked_add(Duration::from_millis(policy.max_retry_delay_ms))
        .ok_or(Error::UpstreamTimeout)?;

    for attempt in 0..max_attempts {
        if attempt > 0 && Instant::now() >= deadline {
            return Err(Error::UpstreamTimeout);
        }

        match execute_one_attempt(client, prepared, request).await {
            Ok(response) => return Ok(response),
            Err(err) => {
                if !should_retry(&err, policy) || attempt + 1 >= max_attempts {
                    return Err(err);
                }

                let delay = {
                    let mut rng = rand::rng();
                    compute_retry_delay_with_rng(&err, policy, attempt, SystemTime::now(), &mut rng)
                };

                let now = Instant::now();
                if sleep_would_exceed_deadline(now, delay, deadline) {
                    return Err(Error::UpstreamTimeout);
                }

                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(Error::UpstreamTimeout)
}

fn should_retry(err: &Error, policy: &EndpointRetryPolicy) -> bool {
    match err {
        Error::UpstreamTimeout => policy.retry_on_timeout,
        Error::UpstreamUnreachable(_) => policy.retry_on_io_errors,
        Error::UpstreamNonSuccess { status, .. } => policy.retry_on_status.contains(status),
        Error::InvalidConfig(_)
        | Error::InvalidRequest(_)
        | Error::ResponseTooLarge { .. }
        | Error::Internal(_) => false,
    }
}

fn compute_retry_delay_with_rng(
    err: &Error,
    policy: &EndpointRetryPolicy,
    attempt: usize,
    now: SystemTime,
    rng: &mut impl Rng,
) -> Duration {
    let exp_max_ms = exponential_backoff_ms(attempt, policy.base_backoff_ms, policy.max_backoff_ms);
    let mut delay = Duration::from_millis(full_jitter_ms(exp_max_ms, rng));

    if let Error::UpstreamNonSuccess {
        status: 429,
        retry_after,
        ..
    } = err
    {
        let rate_limit_delay = Duration::from_millis(policy.rate_limit_backoff_ms);
        delay = if policy.respect_retry_after {
            retry_after
                .as_deref()
                .and_then(|value| parse_retry_after(value, now))
                .unwrap_or(rate_limit_delay)
        } else {
            rate_limit_delay
        };
    }

    let cap = Duration::from_millis(policy.max_retry_delay_ms);
    if delay > cap { cap } else { delay }
}

fn exponential_backoff_ms(attempt: usize, base_ms: u64, cap_ms: u64) -> u64 {
    let exponent = u32::try_from(attempt).unwrap_or(u32::MAX);
    let factor = 2u64.saturating_pow(exponent);
    base_ms.saturating_mul(factor).min(cap_ms)
}

fn full_jitter_ms(max_ms: u64, rng: &mut impl Rng) -> u64 {
    rng.random_range(0..=max_ms)
}

fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(seconds) = trimmed.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let at = httpdate::parse_http_date(trimmed).ok()?;
    match at.duration_since(now) {
        Ok(delay) => Some(delay),
        Err(_) => Some(Duration::ZERO),
    }
}

fn sleep_would_exceed_deadline(now: Instant, delay: Duration, deadline: Instant) -> bool {
    match now.checked_add(delay) {
        Some(next) => next > deadline,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};
    use std::collections::BTreeMap;

    #[test]
    fn exponential_backoff_respects_cap() {
        assert_eq!(exponential_backoff_ms(0, 200, 5_000), 200);
        assert_eq!(exponential_backoff_ms(1, 200, 5_000), 400);
        assert_eq!(exponential_backoff_ms(2, 200, 5_000), 800);
        assert_eq!(exponential_backoff_ms(6, 200, 5_000), 5_000);
    }

    #[test]
    fn full_jitter_bounded() {
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..100 {
            let value = full_jitter_ms(250, &mut rng);
            assert!(value <= 250);
        }
    }

    #[test]
    fn retry_after_header_parsed_as_seconds() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let parsed = parse_retry_after("2", now).unwrap();
        assert_eq!(parsed, Duration::from_secs(2));
    }

    #[test]
    fn retry_after_header_parsed_as_http_date() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let at = now + Duration::from_secs(3);
        let header = httpdate::fmt_http_date(at);

        let parsed = parse_retry_after(&header, now).unwrap();
        assert!(parsed >= Duration::from_secs(2));
        assert!(parsed <= Duration::from_secs(3));
    }

    #[test]
    fn overall_deadline_breaks_retry_loop() {
        let now = Instant::now();
        let deadline = now.checked_add(Duration::from_millis(50)).unwrap();

        assert!(sleep_would_exceed_deadline(
            now,
            Duration::from_millis(51),
            deadline
        ));
        assert!(!sleep_would_exceed_deadline(
            now,
            Duration::from_millis(50),
            deadline
        ));
    }

    #[test]
    fn non_retryable_status_not_retried() {
        let policy = EndpointRetryPolicy {
            retry_on_status: vec![500],
            ..EndpointRetryPolicy::default()
        };

        let err = Error::UpstreamNonSuccess {
            status: 404,
            headers: BTreeMap::new(),
            body: serde_json::json!(null),
            retry_after: None,
        };

        assert!(!should_retry(&err, &policy));
    }
}
