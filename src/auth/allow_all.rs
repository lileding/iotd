use async_trait::async_trait;

use crate::auth::traits::{AuthError, Authenticator, Authorizer, Credentials};

#[derive(Debug)]
pub struct AllowAllAuthenticator;

#[async_trait]
impl Authenticator for AllowAllAuthenticator {
    async fn authenticate(&self, _credentials: &Credentials) -> Result<(), AuthError> {
        Ok(())
    }
}

#[async_trait]
impl Authorizer for AllowAllAuthenticator {
    async fn authorize_publish(&self, _client_id: &str, _topic: &str) -> Result<(), AuthError> {
        Ok(())
    }

    async fn authorize_subscribe(
        &self,
        _client_id: &str,
        _topic_filter: &str,
    ) -> Result<(), AuthError> {
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

    #[tokio::test]
    async fn test_allow_all_authorizes_publish() {
        let auth = AllowAllAuthenticator;
        auth.authorize_publish("any_client", "any/topic")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_allow_all_authorizes_subscribe() {
        let auth = AllowAllAuthenticator;
        auth.authorize_subscribe("any_client", "any/topic/#")
            .await
            .unwrap();
    }
}
