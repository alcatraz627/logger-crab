use async_trait::async_trait;

use crate::{Message, NotifyError, Sink};

pub struct SlackSink {
    pub webhook_url: String,
    pub enabled: bool,
}

#[async_trait]
impl Sink for SlackSink {
    async fn send(&self, _msg: &Message) -> Result<(), NotifyError> {
        if !self.enabled {
            return Err(NotifyError::Disabled);
        }
        // TODO Phase 7.2: POST to self.webhook_url
        Ok(())
    }

    fn name(&self) -> &'static str {
        "slack"
    }
}
