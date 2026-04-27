//! Common error mapping and handling utilities

use anyhow::Result;
use std::error::Error;

/// Convert any error to a user-friendly string message
pub fn error_to_string<E: std::fmt::Display>(err: &E) -> String {
    err.to_string()
}

/// Wrap an error with additional context using anyhow
pub fn wrap_error<E: std::error::Error + Send + Sync + 'static>(
    err: E,
    context: String,
) -> anyhow::Error {
    anyhow::anyhow!(err).context(context)
}

/// Check if an error is a specific type
pub fn is_error_type<E: Error + Send + Sync + 'static>(err: &anyhow::Error) -> bool {
    err.downcast_ref::<E>().is_some()
}

/// Convert a Result to an Option, logging errors
pub fn result_to_option_log<T, E: std::fmt::Display>(
    result: Result<T, E>,
    context: &str,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("{}: {}", context, e);
            None
        }
    }
}

/// Retry an operation with exponential backoff
pub async fn retry_with_backoff<F, Fut, T>(
    mut operation: F,
    max_retries: u32,
    initial_delay_ms: u64,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut delay = initial_delay_ms;

    for attempt in 0..=max_retries {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) if attempt < max_retries => {
                eprintln!(
                    "Attempt {} failed: {}. Retrying in {}ms...",
                    attempt + 1,
                    e,
                    delay
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                delay *= 2;
            }
            Err(e) => return Err(e),
        }
    }

    anyhow::bail!("Max retries exceeded")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_to_string() {
        let err = anyhow::anyhow!("test error");
        let msg = error_to_string(&err);
        assert!(msg.contains("test error"));
    }

    #[test]
    fn test_wrap_error() {
        use std::io;
        let err = io::Error::new(io::ErrorKind::NotFound, "test error");
        let wrapped = wrap_error(err, "context".to_string());
        assert!(wrapped.to_string().contains("context"));
    }

    #[tokio::test]
    async fn test_retry_with_backoff_success() {
        let mut attempts = 0;
        let result = retry_with_backoff(
            || {
                attempts += 1;
                async move {
                    if attempts < 3 {
                        Err::<(), anyhow::Error>(anyhow::anyhow!("not yet"))
                    } else {
                        Ok(())
                    }
                }
            },
            5,
            10,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(attempts, 3);
    }
}
