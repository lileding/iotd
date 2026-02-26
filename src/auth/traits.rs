use async_trait::async_trait;
use bytes::Bytes;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Not authorized")]
    NotAuthorized,
    #[error("Backend error: {0}")]
    BackendError(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: Option<String>,
    pub password: Option<Bytes>,
    pub client_id: String,
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    /// Authenticate a client connection.
    /// Returns Ok(()) on success, Err(AuthError::InvalidCredentials) on failure.
    async fn authenticate(&self, credentials: &Credentials) -> Result<(), AuthError>;
}

#[async_trait]
pub trait Authorizer: Send + Sync {
    async fn authorize_publish(&self, client_id: &str, topic: &str) -> Result<(), AuthError>;
    async fn authorize_subscribe(
        &self,
        client_id: &str,
        topic_filter: &str,
    ) -> Result<(), AuthError>;
}
