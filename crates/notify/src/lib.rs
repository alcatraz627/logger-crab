//! Pluggable notification sinks. V1 ships with stubs; real Slack + SES
//! impls arrive in Phase 7.2 / 7.5.

use async_trait::async_trait;

pub mod ses;
pub mod slack;

#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("sink disabled")]
    Disabled,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub title: String,
    pub body: String,
    pub link: Option<String>,
}

#[async_trait]
pub trait Sink: Send + Sync {
    async fn send(&self, msg: &Message) -> Result<(), NotifyError>;
    fn name(&self) -> &'static str;
}
