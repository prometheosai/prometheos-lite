//! Async task utilities for spawning and logging failures

use anyhow::Result;
use std::future::Future;
use tokio::task::JoinHandle;

/// Spawn a task that logs errors on failure
pub fn spawn_task_log_error<F>(task: F, task_name: &str) -> JoinHandle<()>
where
    F: Future<Output = Result<()>> + Send + 'static,
{
    let name = task_name.to_string();
    tokio::spawn(async move {
        if let Err(e) = task.await {
            eprintln!("Task '{}' failed: {}", name, e);
        }
    })
}

/// Spawn a task that ignores errors
pub fn spawn_task_ignore_error<F>(task: F) -> JoinHandle<()>
where
    F: Future<Output = Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        let _ = task.await;
    })
}

/// Spawn a task with a timeout
pub async fn spawn_with_timeout<F, T>(task: F, timeout_ms: u64, task_name: &str) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), task).await {
        Ok(result) => result,
        Err(_) => {
            anyhow::bail!("Task '{}' timed out after {}ms", task_name, timeout_ms)
        }
    }
}

/// Run multiple tasks concurrently and collect results
pub async fn run_concurrent<T, E>(
    tasks: Vec<impl Future<Output = Result<T, E>> + Send>,
) -> Vec<Result<T, E>>
where
    T: Send,
    E: Send,
{
    futures::future::join_all(tasks).await
}

/// Run tasks in parallel with a concurrency limit
pub async fn run_parallel<T, E, F>(tasks: Vec<F>, max_concurrency: usize) -> Vec<Result<T, E>>
where
    F: Future<Output = Result<T, E>> + Send,
    T: Send,
    E: Send,
{
    use futures::stream::{self, StreamExt};

    stream::iter(tasks)
        .map(|task| task)
        .buffer_unordered(max_concurrency)
        .collect()
        .await
}

/// Spawn a background task that can be cancelled via a handle
pub fn spawn_cancellable<F>(task: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(task)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_with_timeout_success() {
        let result = spawn_with_timeout(async { Ok::<(), anyhow::Error>(()) }, 1000, "test").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_spawn_with_timeout_failure() {
        let result = spawn_with_timeout(
            std::future::pending::<Result<(), anyhow::Error>>(),
            1,
            "test",
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_concurrent() {
        let tasks = vec![
            futures::future::ready(Ok::<u32, anyhow::Error>(1)),
            futures::future::ready(Ok::<u32, anyhow::Error>(2)),
        ];

        let results = run_concurrent(tasks).await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].as_ref().ok().copied(), Some(1));
        assert_eq!(results[1].as_ref().ok().copied(), Some(2));
    }
}
