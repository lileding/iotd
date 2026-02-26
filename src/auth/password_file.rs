use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;

use crate::auth::traits::{AuthError, Authenticator, Credentials};

#[derive(Debug)]
pub struct PasswordFileAuthenticator {
    credentials: HashMap<String, String>,
}

impl PasswordFileAuthenticator {
    pub fn new(path: &Path) -> Result<Self, AuthError> {
        let contents = std::fs::read_to_string(path)?;
        let mut credentials = HashMap::new();

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((username, password)) = line.split_once(':') {
                credentials.insert(username.to_string(), password.to_string());
            }
        }

        Ok(Self { credentials })
    }
}

#[async_trait]
impl Authenticator for PasswordFileAuthenticator {
    async fn authenticate(&self, creds: &Credentials) -> Result<(), AuthError> {
        let username = creds
            .username
            .as_deref()
            .ok_or(AuthError::InvalidCredentials)?;
        let password = creds
            .password
            .as_ref()
            .ok_or(AuthError::InvalidCredentials)?;

        let password_str =
            std::str::from_utf8(password).map_err(|_| AuthError::InvalidCredentials)?;

        match self.credentials.get(username) {
            Some(stored) if stored == password_str => Ok(()),
            _ => Err(AuthError::InvalidCredentials),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::io::Write;

    fn create_temp_password_file(content: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    #[tokio::test]
    async fn test_valid_credentials() {
        let file = create_temp_password_file("admin:secret\nuser1:pass1\n");
        let auth = PasswordFileAuthenticator::new(file.path()).unwrap();

        let creds = Credentials {
            username: Some("admin".to_string()),
            password: Some(Bytes::from("secret")),
            client_id: "test".to_string(),
        };
        auth.authenticate(&creds).await.unwrap();
    }

    #[tokio::test]
    async fn test_invalid_password() {
        let file = create_temp_password_file("admin:secret\n");
        let auth = PasswordFileAuthenticator::new(file.path()).unwrap();

        let creds = Credentials {
            username: Some("admin".to_string()),
            password: Some(Bytes::from("wrong")),
            client_id: "test".to_string(),
        };
        assert!(matches!(
            auth.authenticate(&creds).await,
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn test_unknown_username() {
        let file = create_temp_password_file("admin:secret\n");
        let auth = PasswordFileAuthenticator::new(file.path()).unwrap();

        let creds = Credentials {
            username: Some("unknown".to_string()),
            password: Some(Bytes::from("secret")),
            client_id: "test".to_string(),
        };
        assert!(matches!(
            auth.authenticate(&creds).await,
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn test_no_username() {
        let file = create_temp_password_file("admin:secret\n");
        let auth = PasswordFileAuthenticator::new(file.path()).unwrap();

        let creds = Credentials {
            username: None,
            password: Some(Bytes::from("secret")),
            client_id: "test".to_string(),
        };
        assert!(matches!(
            auth.authenticate(&creds).await,
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn test_no_password() {
        let file = create_temp_password_file("admin:secret\n");
        let auth = PasswordFileAuthenticator::new(file.path()).unwrap();

        let creds = Credentials {
            username: Some("admin".to_string()),
            password: None,
            client_id: "test".to_string(),
        };
        assert!(matches!(
            auth.authenticate(&creds).await,
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn test_comments_and_empty_lines() {
        let file =
            create_temp_password_file("# This is a comment\n\nadmin:secret\n# Another comment\n");
        let auth = PasswordFileAuthenticator::new(file.path()).unwrap();

        let creds = Credentials {
            username: Some("admin".to_string()),
            password: Some(Bytes::from("secret")),
            client_id: "test".to_string(),
        };
        auth.authenticate(&creds).await.unwrap();
    }
}
