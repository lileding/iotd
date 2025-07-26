use async_trait::async_trait;
use bytes::Bytes;

#[async_trait]
pub trait MessageStorage: Send + Sync {
    async fn store_message(
        &self,
        topic: &str,
        payload: &Bytes,
    ) -> Result<(), Box<dyn std::error::Error>>;
    async fn retrieve_messages(
        &self,
        topic_filter: &str,
    ) -> Result<Vec<(String, Bytes)>, Box<dyn std::error::Error>>;
    async fn delete_messages(&self, topic: &str) -> Result<(), Box<dyn std::error::Error>>;
}

#[async_trait]
pub trait SessionStorage: Send + Sync {
    async fn store_session(
        &self,
        client_id: &str,
        session_data: &[u8],
    ) -> Result<(), Box<dyn std::error::Error>>;
    async fn retrieve_session(
        &self,
        client_id: &str,
    ) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>>;
    async fn delete_session(&self, client_id: &str) -> Result<(), Box<dyn std::error::Error>>;
}
