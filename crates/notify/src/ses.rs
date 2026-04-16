use async_trait::async_trait;

use crate::{Message, NotifyError, Sink};

pub struct SesSink {
    pub region: String,
    pub from: String,
    pub to: Vec<String>,
    pub enabled: bool,
}

#[async_trait]
impl Sink for SesSink {
    async fn send(&self, _msg: &Message) -> Result<(), NotifyError> {
        if !self.enabled {
            return Err(NotifyError::Disabled);
        }
        // TODO Phase 7.5: build aws-sdk-sesv2 client and send
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ses"
    }
}
