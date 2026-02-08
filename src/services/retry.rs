use std::time::Duration;
use tokio::time::sleep;

/// Retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub exponential_base: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            exponential_base: 2.0,
        }
    }
}

/// Retry a future with exponential backoff
pub async fn retry_with_backoff<F, Fut, T, E>(
    config: &RetryConfig,
    operation_name: &str,
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_error = None;
    
    for attempt in 1..=config.max_attempts {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let error_msg = e.to_string();
                log::warn!(
                    "{} failed (attempt {}/{}): {}",
                    operation_name,
                    attempt,
                    config.max_attempts,
                    error_msg
                );
                
                last_error = Some(e);
                
                if attempt < config.max_attempts {
                    let delay = calculate_delay(config, attempt);
                    log::info!("Retrying {} in {:?}...", operation_name, delay);
                    sleep(delay).await;
                }
            }
        }
    }
    
    Err(last_error.expect("At least one attempt was made"))
}

/// Calculate delay with exponential backoff and jitter
fn calculate_delay(config: &RetryConfig, attempt: u32) -> Duration {
    let exponential = config.exponential_base.powi((attempt - 1) as i32);
    let delay_ms = (config.base_delay.as_millis() as f64 * exponential) as u64;
    
    // Add jitter (±25%)
    let jitter = (delay_ms as f64 * 0.25) as u64;
    let jittered = delay_ms + rand::random::<u64>() % (jitter * 2 + 1) - jitter;
    
    let final_delay = jittered.min(config.max_delay.as_millis() as u64);
    
    Duration::from_millis(final_delay)
}

/// Retry policy for specific error types
#[derive(Debug, Clone)]
pub enum RetryDecision {
    Retry,
    Abort,
}

pub async fn retry_with_policy<F, Fut, T, E>(
    config: &RetryConfig,
    operation_name: &str,
    mut f: F,
    policy: impl Fn(&E) -> RetryDecision,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_error = None;
    
    for attempt in 1..=config.max_attempts {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                match policy(&e) {
                    RetryDecision::Abort => {
                        log::error!("{} aborted: {}", operation_name, e);
                        return Err(e);
                    }
                    RetryDecision::Retry => {
                        log::warn!(
                            "{} failed (attempt {}/{}): {}",
                            operation_name,
                            attempt,
                            config.max_attempts,
                            e
                        );
                        
                        last_error = Some(e);
                        
                        if attempt < config.max_attempts {
                            let delay = calculate_delay(config, attempt);
                            sleep(delay).await;
                        }
                    }
                }
            }
        }
    }
    
    Err(last_error.expect("At least one attempt was made"))
}

/// Circuit breaker pattern
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    failure_threshold: u32,
    reset_timeout: Duration,
    state: CircuitState,
    failures: u32,
    last_failure: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            failure_threshold,
            reset_timeout,
            state: CircuitState::Closed,
            failures: 0,
            last_failure: None,
        }
    }
    
    pub fn can_execute(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if let Some(last) = self.last_failure {
                    if last.elapsed() >= self.reset_timeout {
                        self.state = CircuitState::HalfOpen;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }
    
    pub fn record_success(&mut self) {
        self.failures = 0;
        self.state = CircuitState::Closed;
    }
    
    pub fn record_failure(&mut self) {
        self.failures += 1;
        self.last_failure = Some(std::time::Instant::now());
        
        if self.failures >= self.failure_threshold {
            self.state = CircuitState::Open;
        }
    }
    
    pub fn is_open(&self) -> bool {
        self.state == CircuitState::Open
    }
}
