use actix::prelude::*;
use actix_web::{web, Error, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::services::background::{JobManager, JobStatus, BackgroundJob};

/// WebSocket session for job progress
pub struct JobWebSocket {
    /// Client ID
    id: String,
    /// Last ping time
    hb: Instant,
    /// Job manager reference
    job_manager: Arc<JobManager>,
    /// Currently monitored job IDs
    watched_jobs: Vec<String>,
    /// Heartbeat interval
    hb_interval: Duration,
    /// Job poll interval
    poll_interval: Duration,
}

/// Messages sent to WebSocket clients
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// Job status update
    #[serde(rename = "job_update")]
    JobUpdate { job_id: String, status: JobStatusWs },
    
    /// Connected confirmation
    #[serde(rename = "connected")]
    Connected { client_id: String, message: String },
    
    /// Error message
    #[serde(rename = "error")]
    Error { message: String },
    
    /// Pong response
    #[serde(rename = "pong")]
    Pong { timestamp: i64 },
}

/// Job status for WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatusWs {
    pub state: String,
    pub progress: Option<f32>,
    pub message: Option<String>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl From<&JobStatus> for JobStatusWs {
    fn from(status: &JobStatus) -> Self {
        match status {
            JobStatus::Pending => JobStatusWs {
                state: "pending".to_string(),
                progress: None,
                message: None,
                result: None,
                error: None,
            },
            JobStatus::Running { progress, message } => JobStatusWs {
                state: "running".to_string(),
                progress: Some(*progress),
                message: Some(message.clone()),
                result: None,
                error: None,
            },
            JobStatus::Completed { result } => JobStatusWs {
                state: "completed".to_string(),
                progress: Some(100.0),
                message: Some("Done".to_string()),
                result: Some(result.clone()),
                error: None,
            },
            JobStatus::Failed { error } => JobStatusWs {
                state: "failed".to_string(),
                progress: None,
                message: None,
                result: None,
                error: Some(error.clone()),
            },
            JobStatus::Cancelled => JobStatusWs {
                state: "cancelled".to_string(),
                progress: None,
                message: None,
                result: None,
                error: None,
            },
        }
    }
}

impl JobWebSocket {
    pub fn new(job_manager: Arc<JobManager>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            hb: Instant::now(),
            job_manager,
            watched_jobs: Vec::new(),
            hb_interval: Duration::from_secs(5),
            poll_interval: Duration::from_millis(500),
        }
    }

    /// Send message to client
    fn send_message(&self, ctx: &mut ws::WebsocketContext<Self>, msg: WsMessage) {
        let text = serde_json::to_string(&msg).unwrap_or_default();
        ctx.text(text);
    }

    /// Send current status of watched jobs
    fn poll_jobs(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        for job_id in &self.watched_jobs {
            if let Some(job) = futures::executor::block_on(self.job_manager.get_job(job_id)) {
                self.send_message(ctx, WsMessage::JobUpdate {
                    job_id: job_id.clone(),
                    status: JobStatusWs::from(&job.status),
                });
            }
        }
    }

    /// Handle client commands
    fn handle_command(&mut self, text: &str, ctx: &mut ws::WebsocketContext<Self>) {
        #[derive(Deserialize)]
        struct Command {
            action: String,
            job_id: Option<String>,
        }

        match serde_json::from_str::<Command>(text) {
            Ok(cmd) => {
                match cmd.action.as_str() {
                    "watch" => {
                        if let Some(job_id) = cmd.job_id {
                            if !self.watched_jobs.contains(&job_id) {
                                self.watched_jobs.push(job_id.clone());
                                log::info!("Client {} started watching job {}", self.id, job_id);
                                
                                // Send immediate update
                                if let Some(job) = futures::executor::block_on(self.job_manager.get_job(&job_id)) {
                                    self.send_message(ctx, WsMessage::JobUpdate {
                                        job_id: job_id.clone(),
                                        status: JobStatusWs::from(&job.status),
                                    });
                                }
                            }
                        }
                    }
                    "unwatch" => {
                        if let Some(job_id) = cmd.job_id {
                            self.watched_jobs.retain(|id| id != &job_id);
                            log::info!("Client {} stopped watching job {}", self.id, job_id);
                        }
                    }
                    "watch_all" => {
                        // Get all jobs and watch them
                        let jobs = futures::executor::block_on(self.job_manager.list_jobs());
                        for job in jobs {
                            if !self.watched_jobs.contains(&job.id) {
                                self.watched_jobs.push(job.id.clone());
                            }
                        }
                    }
                    "ping" => {
                        self.send_message(ctx, WsMessage::Pong {
                            timestamp: chrono::Utc::now().timestamp_millis(),
                        });
                    }
                    _ => {
                        self.send_message(ctx, WsMessage::Error {
                            message: format!("Unknown action: {}", cmd.action),
                        });
                    }
                }
            }
            Err(e) => {
                self.send_message(ctx, WsMessage::Error {
                    message: format!("Invalid command: {}", e),
                });
            }
        }
    }
}

impl Actor for JobWebSocket {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Start heartbeat
        self.hb(ctx);
        
        // Start job polling
        let poll_interval = self.poll_interval;
        ctx.run_interval(poll_interval, |act, ctx| {
            act.poll_jobs(ctx);
        });

        // Send connected message
        self.send_message(ctx, WsMessage::Connected {
            client_id: self.id.clone(),
            message: "Connected to job progress WebSocket".to_string(),
        });

        log::info!("WebSocket client {} connected", self.id);
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        log::info!("WebSocket client {} disconnected", self.id);
    }
}

/// Handler for heartbeat
impl JobWebSocket {
    fn hb(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(self.hb_interval, |act, ctx| {
            if Instant::now().duration_since(act.hb) > Duration::from_secs(30) {
                log::info!("WebSocket client {} timed out", act.id);
                ctx.stop();
                return;
            }
            ctx.ping(b"");
        });
    }
}

/// Stream handler for WebSocket messages
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for JobWebSocket {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.hb = Instant::now();
            }
            Ok(ws::Message::Text(text)) => {
                self.handle_command(&text, ctx);
            }
            Ok(ws::Message::Binary(bin)) => {
                log::debug!("Received binary message: {} bytes", bin.len());
            }
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => (),
        }
    }
}

/// WebSocket handler
pub async fn job_websocket(
    req: HttpRequest,
    stream: web::Payload,
    job_manager: web::Data<Arc<JobManager>>,
) -> Result<HttpResponse, Error> {
    ws::start(
        JobWebSocket::new(job_manager.get_ref().clone()),
        &req,
        stream,
    )
}

/// Broadcast job update to all connected clients (if needed)
pub async fn broadcast_job_update(
    job_manager: &JobManager,
    job_id: &str,
) {
    // This would require storing actor addresses globally
    // For now, clients poll for updates
    log::debug!("Job {} updated, clients will receive on next poll", job_id);
}
