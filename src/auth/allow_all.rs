use async_trait::async_trait;

use crate::auth::traits::{AuthError, Authenticator, Credentials};

#[derive(Debug)]
pub struct AllowAllAuthenticator;

#[async_trait]
impl Authenticator for AllowAllAuthenticator {
    async fn authenticate(&self, _credentials: &Credentials) -> Result<(), AuthError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[tokio::test]
    async fn test_allow_all_accepts_no_credentials() {
        let auth = AllowAllAuthenticator;
        let creds = Credentials {
            username: None,
            password: None,
            client_id: "test".to_string(),
        };
        auth.authenticate(&creds).await.unwrap();
    }

    #[tokio::test]
    async fn test_allow_all_accepts_with_credentials() {
        let auth = AllowAllAuthenticator;
        let creds = Credentials {
            username: Some("user".to_string()),
            password: Some(Bytes::from("pass")),
            client_id: "test".to_string(),
        };
        auth.authenticate(&creds).await.unwrap();
    }
}
