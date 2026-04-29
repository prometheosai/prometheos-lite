//! Job queue for async execution
//!
//! This module provides a job queue system for managing asynchronous task execution
//! with priority, retry logic, and status tracking.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

/// Job status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Job priority
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Job - represents a unit of work to be executed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// Unique identifier
    pub id: String,
    /// Job type/name
    pub job_type: String,
    /// Job payload/data
    pub payload: serde_json::Value,
    /// Job priority
    pub priority: JobPriority,
    /// Current status
    pub status: JobStatus,
    /// Number of retry attempts
    pub retry_count: u32,
    /// Maximum retry attempts
    pub max_retries: u32,
    /// Error message if failed
    pub error: Option<String>,
    /// Result if completed
    pub result: Option<serde_json::Value>,
    /// Created at timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Started at timestamp
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Completed at timestamp
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Job {
    pub fn new(job_type: String, payload: serde_json::Value, priority: JobPriority) -> Self {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        Self {
            id,
            job_type,
            payload,
            priority,
            status: JobStatus::Pending,
            retry_count: 0,
            max_retries: 3,
            error: None,
            result: None,
            created_at: now,
            started_at: None,
            completed_at: None,
        }
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }
}

/// Job executor trait - defines how jobs are executed
#[async_trait::async_trait]
pub trait JobExecutor: Send + Sync {
    async fn execute(&self, job: &Job) -> Result<serde_json::Value>;
}

/// Job queue - manages job execution
pub struct JobQueue {
    jobs: Arc<RwLock<HashMap<String, Job>>>,
    pending_jobs: Arc<Mutex<Vec<String>>>,
    executor: Arc<dyn JobExecutor>,
    max_concurrent: usize,
    running_count: Arc<Mutex<usize>>,
}

impl JobQueue {
    pub fn new(executor: Arc<dyn JobExecutor>, max_concurrent: usize) -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            pending_jobs: Arc::new(Mutex::new(Vec::new())),
            executor,
            max_concurrent,
            running_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Submit a new job to the queue
    pub async fn submit(&self, job: Job) -> Result<String> {
        let job_id = job.id.clone();
        
        // Add to jobs map
        {
            let mut jobs = self.jobs.write().await;
            jobs.insert(job_id.clone(), job);
        }

        // Add to pending queue
        {
            let mut pending = self.pending_jobs.lock().await;
            pending.push(job_id.clone());
        }

        // Sort by priority (highest first) - do this outside the lock
        {
            let jobs = self.jobs.read().await;
            let mut pending = self.pending_jobs.lock().await;
            pending.sort_by(|a, b| {
                let job_a = jobs.get(a);
                let job_b = jobs.get(b);
                match (job_a, job_b) {
                    (Some(a), Some(b)) => b.priority.cmp(&a.priority),
                    _ => std::cmp::Ordering::Equal,
                }
            });
        }

        // Try to process pending jobs
        self.process_queue().await?;

        Ok(job_id)
    }

    /// Get job status
    pub async fn get_job(&self, job_id: &str) -> Option<Job> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id).cloned()
    }

    /// List all jobs
    pub async fn list_jobs(&self) -> Vec<Job> {
        let jobs = self.jobs.read().await;
        jobs.values().cloned().collect()
    }

    /// Cancel a job
    pub async fn cancel_job(&self, job_id: &str) -> Result<()> {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            if job.status == JobStatus::Pending {
                job.status = JobStatus::Cancelled;
                job.completed_at = Some(chrono::Utc::now());
                
                // Remove from pending queue
                let mut pending = self.pending_jobs.lock().await;
                pending.retain(|id| id != job_id);
            }
        }
        Ok(())
    }

    /// Process pending jobs
    async fn process_queue(&self) -> Result<()> {
        loop {
            // Check if we can start more jobs
            {
                let running = self.running_count.lock().await;
                if *running >= self.max_concurrent {
                    break;
                }
            }

            // Get next pending job
            let job_id = {
                let mut pending = self.pending_jobs.lock().await;
                if pending.is_empty() {
                    break;
                }
                pending.remove(0)
            };

            // Update job status to running
            {
                let mut jobs = self.jobs.write().await;
                if let Some(job) = jobs.get_mut(&job_id) {
                    job.status = JobStatus::Running;
                    job.started_at = Some(chrono::Utc::now());
                }
            }

            // Increment running count
            {
                let mut running = self.running_count.lock().await;
                *running += 1;
            }

            // Spawn task to execute job
            let jobs = self.jobs.clone();
            let executor = self.executor.clone();
            let running_count = self.running_count.clone();
            let job_id_clone = job_id.clone();

            tokio::spawn(async move {
                let result = executor.execute(&jobs.read().await.get(&job_id_clone).unwrap()).await;

                let mut jobs = jobs.write().await;
                if let Some(job) = jobs.get_mut(&job_id_clone) {
                    match result {
                        Ok(output) => {
                            job.status = JobStatus::Completed;
                            job.result = Some(output);
                            job.completed_at = Some(chrono::Utc::now());
                        }
                        Err(e) => {
                            job.retry_count += 1;
                            if job.retry_count >= job.max_retries {
                                job.status = JobStatus::Failed;
                                job.error = Some(e.to_string());
                                job.completed_at = Some(chrono::Utc::now());
                            } else {
                                job.status = JobStatus::Pending;
                                job.started_at = None;
                            }
                        }
                    }
                }

                // Decrement running count
                let mut running = running_count.lock().await;
                *running -= 1;
            });
        }

        Ok(())
    }

    /// Get queue statistics
    pub async fn stats(&self) -> JobQueueStats {
        let jobs = self.jobs.read().await;
        let pending = self.pending_jobs.lock().await;
        let running = *self.running_count.lock().await;

        let mut stats = JobQueueStats {
            total: jobs.len(),
            pending: pending.len(),
            running,
            completed: 0,
            failed: 0,
            cancelled: 0,
        };

        for job in jobs.values() {
            match job.status {
                JobStatus::Completed => stats.completed += 1,
                JobStatus::Failed => stats.failed += 1,
                JobStatus::Cancelled => stats.cancelled += 1,
                _ => {}
            }
        }

        stats
    }
}

/// Job queue statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobQueueStats {
    pub total: usize,
    pub pending: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
}

/// Simple in-memory job executor for testing
pub struct SimpleJobExecutor;

#[async_trait::async_trait]
impl JobExecutor for SimpleJobExecutor {
    async fn execute(&self, job: &Job) -> Result<serde_json::Value> {
        // Simulate work
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        Ok(serde_json::json!({
            "job_id": job.id,
            "job_type": job.job_type,
            "status": "completed",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_job_creation() {
        let job = Job::new(
            "test_job".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );

        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.retry_count, 0);
        assert_eq!(job.max_retries, 3);
    }

    #[tokio::test]
    async fn test_job_queue_submit() {
        let executor = Arc::new(SimpleJobExecutor);
        let queue = JobQueue::new(executor, 2);

        let job = Job::new(
            "test_job".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );

        let job_id = queue.submit(job).await.unwrap();
        assert!(!job_id.is_empty());

        // Wait for job to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let job = queue.get_job(&job_id).await;
        assert!(job.is_some());
        assert_eq!(job.unwrap().status, JobStatus::Completed);
    }

    #[tokio::test]
    async fn test_job_queue_priority() {
        let executor = Arc::new(SimpleJobExecutor);
        let queue = JobQueue::new(executor, 1);

        let job1 = Job::new(
            "low_priority".to_string(),
            serde_json::json!({}),
            JobPriority::Low,
        );

        let job2 = Job::new(
            "high_priority".to_string(),
            serde_json::json!({}),
            JobPriority::High,
        );

        queue.submit(job1).await.unwrap();
        queue.submit(job2).await.unwrap();

        let pending = queue.pending_jobs.lock().await;
        // High priority should be first
        assert_eq!(pending[0], "high_priority");
    }

    #[tokio::test]
    async fn test_job_cancel() {
        let executor = Arc::new(SimpleJobExecutor);
        let queue = JobQueue::new(executor, 0); // No concurrent execution

        let job = Job::new(
            "test_job".to_string(),
            serde_json::json!({}),
            JobPriority::Normal,
        );

        let job_id = queue.submit(job).await.unwrap();
        queue.cancel_job(&job_id).await.unwrap();

        let job = queue.get_job(&job_id).await.unwrap();
        assert_eq!(job.status, JobStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_job_queue_stats() {
        let executor = Arc::new(SimpleJobExecutor);
        let queue = JobQueue::new(executor, 2);

        let job = Job::new(
            "test_job".to_string(),
            serde_json::json!({}),
            JobPriority::Normal,
        );

        queue.submit(job).await.unwrap();

        let stats = queue.stats().await;
        assert_eq!(stats.total, 1);
    }
}
