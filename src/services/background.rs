use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Background job status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobStatus {
    Pending,
    Running { progress: f32, message: String },
    Completed { result: serde_json::Value },
    Failed { error: String },
    Cancelled,
}

/// Background job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundJob {
    pub id: String,
    pub job_type: JobType,
    pub status: JobStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobType {
    BatchOcr {
        book_id: String,
        page_range: (u32, u32),
        chapter_id: String,
    },
    BatchSolve {
        problem_ids: Vec<String>,
        provider: String,
    },
    Export {
        book_id: String,
        format: ExportFormat,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExportFormat {
    Markdown,
    Latex,
    Json,
    Anki,
}

/// Background job manager
#[derive(Clone)]
pub struct JobManager {
    jobs: Arc<RwLock<HashMap<String, BackgroundJob>>>,
    tx: mpsc::UnboundedSender<JobCommand>,
}

#[derive(Debug)]
enum JobCommand {
    UpdateStatus(String, JobStatus),
    Cancel(String),
}

impl JobManager {
    pub fn new() -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<JobCommand>();
        let jobs: Arc<RwLock<HashMap<String, BackgroundJob>>> = Arc::new(RwLock::new(HashMap::new()));
        let jobs_clone = jobs.clone();
        
        // Background task processor
        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    JobCommand::UpdateStatus(id, status) => {
                        let mut jobs = jobs_clone.write().await;
                        if let Some(job) = jobs.get_mut(&id) {
                            job.status = status;
                            job.updated_at = Utc::now();
                        }
                    }
                    JobCommand::Cancel(id) => {
                        let mut jobs = jobs_clone.write().await;
                        if let Some(job) = jobs.get_mut(&id) {
                            job.status = JobStatus::Cancelled;
                            job.updated_at = Utc::now();
                        }
                    }
                }
            }
        });
        
        Self { jobs, tx }
    }
    
    pub async fn create_job(&self, job_type: JobType) -> String {
        let id = Uuid::new_v4().to_string();
        let job = BackgroundJob {
            id: id.clone(),
            job_type,
            status: JobStatus::Pending,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        
        let mut jobs = self.jobs.write().await;
        jobs.insert(id.clone(), job);
        
        id
    }
    
    pub async fn get_job(&self, id: &str) -> Option<BackgroundJob> {
        let jobs = self.jobs.read().await;
        jobs.get(id).cloned()
    }
    
    pub async fn list_jobs(&self) -> Vec<BackgroundJob> {
        let jobs = self.jobs.read().await;
        jobs.values().cloned().collect()
    }
    
    pub async fn update_progress(&self, id: &str, progress: f32, message: &str) {
        let _ = self.tx.send(JobCommand::UpdateStatus(
            id.to_string(),
            JobStatus::Running {
                progress,
                message: message.to_string(),
            }
        ));
    }
    
    pub async fn complete_job(&self, id: &str, result: serde_json::Value) {
        let _ = self.tx.send(JobCommand::UpdateStatus(
            id.to_string(),
            JobStatus::Completed { result }
        ));
    }
    
    pub async fn fail_job(&self, id: &str, error: &str) {
        let _ = self.tx.send(JobCommand::UpdateStatus(
            id.to_string(),
            JobStatus::Failed { error: error.to_string() }
        ));
    }
    
    pub async fn cancel_job(&self, id: &str) {
        let _ = self.tx.send(JobCommand::Cancel(id.to_string()));
    }
    
    /// Clean up old completed jobs (older than 24 hours)
    pub async fn cleanup_old_jobs(&self) {
        let cutoff = Utc::now() - chrono::Duration::hours(24);
        let mut jobs = self.jobs.write().await;
        jobs.retain(|_, job| {
            match &job.status {
                JobStatus::Completed { .. } | JobStatus::Failed { .. } | JobStatus::Cancelled => {
                    job.updated_at > cutoff
                }
                _ => true,
            }
        });
    }
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}
