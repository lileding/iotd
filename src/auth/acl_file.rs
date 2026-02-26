use async_trait::async_trait;
use std::path::Path;

use crate::auth::traits::{AuthError, Authorizer};

#[derive(Debug, Clone, PartialEq)]
enum Permission {
    Publish,
    Subscribe,
    PublishSubscribe,
}

#[derive(Debug, Clone)]
struct AclRule {
    client_id: String,
    permission: Permission,
    topic_filter: String,
}

#[derive(Debug)]
pub struct AclFileAuthorizer {
    rules: Vec<AclRule>,
}

impl AclFileAuthorizer {
    pub fn new(path: &Path) -> Result<Self, AuthError> {
        let contents = std::fs::read_to_string(path)?;
        let mut rules = Vec::new();

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let parts: Vec<&str> = line.splitn(3, ' ').collect();
            if parts.len() != 3 {
                continue;
            }

            let permission = match parts[1] {
                "pub" => Permission::Publish,
                "sub" => Permission::Subscribe,
                "pubsub" => Permission::PublishSubscribe,
                _ => continue,
            };

            rules.push(AclRule {
                client_id: parts[0].to_string(),
                permission,
                topic_filter: parts[2].to_string(),
            });
        }

        Ok(Self { rules })
    }
}

#[async_trait]
impl Authorizer for AclFileAuthorizer {
    async fn authorize_publish(&self, client_id: &str, topic: &str) -> Result<(), AuthError> {
        for rule in &self.rules {
            if rule.client_id != client_id {
                continue;
            }
            if !matches!(
                rule.permission,
                Permission::Publish | Permission::PublishSubscribe
            ) {
                continue;
            }
            if rule.topic_filter == topic {
                return Ok(());
            }
        }
        Err(AuthError::NotAuthorized)
    }

    async fn authorize_subscribe(
        &self,
        client_id: &str,
        topic_filter: &str,
    ) -> Result<(), AuthError> {
        for rule in &self.rules {
            if rule.client_id != client_id {
                continue;
            }
            if !matches!(
                rule.permission,
                Permission::Subscribe | Permission::PublishSubscribe
            ) {
                continue;
            }
            if rule.topic_filter == topic_filter {
                return Ok(());
            }
        }
        Err(AuthError::NotAuthorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn create_temp_acl_file(content: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    const ACL_CONTENT: &str = "\
# Test ACL file
admin pubsub #
sensor1 pub sensors/data
viewer sub sensors/#
";

    #[tokio::test]
    async fn test_publish_allowed() {
        let file = create_temp_acl_file(ACL_CONTENT);
        let acl = AclFileAuthorizer::new(file.path()).unwrap();

        acl.authorize_publish("admin", "#").await.unwrap();
        acl.authorize_publish("sensor1", "sensors/data")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_publish_denied_wrong_client() {
        let file = create_temp_acl_file(ACL_CONTENT);
        let acl = AclFileAuthorizer::new(file.path()).unwrap();

        assert!(matches!(
            acl.authorize_publish("unknown", "sensors/data").await,
            Err(AuthError::NotAuthorized)
        ));
    }

    #[tokio::test]
    async fn test_publish_denied_wrong_topic() {
        let file = create_temp_acl_file(ACL_CONTENT);
        let acl = AclFileAuthorizer::new(file.path()).unwrap();

        assert!(matches!(
            acl.authorize_publish("sensor1", "other/topic").await,
            Err(AuthError::NotAuthorized)
        ));
    }

    #[tokio::test]
    async fn test_publish_denied_sub_only() {
        let file = create_temp_acl_file(ACL_CONTENT);
        let acl = AclFileAuthorizer::new(file.path()).unwrap();

        assert!(matches!(
            acl.authorize_publish("viewer", "sensors/#").await,
            Err(AuthError::NotAuthorized)
        ));
    }

    #[tokio::test]
    async fn test_subscribe_allowed() {
        let file = create_temp_acl_file(ACL_CONTENT);
        let acl = AclFileAuthorizer::new(file.path()).unwrap();

        acl.authorize_subscribe("admin", "#").await.unwrap();
        acl.authorize_subscribe("viewer", "sensors/#")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_subscribe_denied_pub_only() {
        let file = create_temp_acl_file(ACL_CONTENT);
        let acl = AclFileAuthorizer::new(file.path()).unwrap();

        assert!(matches!(
            acl.authorize_subscribe("sensor1", "sensors/data").await,
            Err(AuthError::NotAuthorized)
        ));
    }

    #[tokio::test]
    async fn test_pubsub_grants_both() {
        let file = create_temp_acl_file(ACL_CONTENT);
        let acl = AclFileAuthorizer::new(file.path()).unwrap();

        acl.authorize_publish("admin", "#").await.unwrap();
        acl.authorize_subscribe("admin", "#").await.unwrap();
    }

    #[tokio::test]
    async fn test_comments_and_empty_lines() {
        let file = create_temp_acl_file("# comment\n\nclient1 pub topic1\n# another comment\n");
        let acl = AclFileAuthorizer::new(file.path()).unwrap();

        acl.authorize_publish("client1", "topic1").await.unwrap();
    }

    #[tokio::test]
    async fn test_invalid_lines_skipped() {
        let file = create_temp_acl_file("badline\nclient1 pub topic1\nclient2 invalid topic2\n");
        let acl = AclFileAuthorizer::new(file.path()).unwrap();

        acl.authorize_publish("client1", "topic1").await.unwrap();
        assert!(matches!(
            acl.authorize_publish("client2", "topic2").await,
            Err(AuthError::NotAuthorized)
        ));
    }
}
