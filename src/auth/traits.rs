use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: Option<String>,
    pub password: Option<String>,
    pub client_id: String,
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, credentials: &Credentials) -> Result<bool, Box<dyn std::error::Error>>;
}

#[async_trait]
pub trait Authorizer: Send + Sync {
    async fn authorize_publish(&self, client_id: &str, topic: &str) -> Result<bool, Box<dyn std::error::Error>>;
    async fn authorize_subscribe(&self, client_id: &str, topic_filter: &str) -> Result<bool, Box<dyn std::error::Error>>;
}