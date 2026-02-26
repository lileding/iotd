pub mod acl_file;
pub mod allow_all;
pub mod password_file;
pub mod traits;

pub use acl_file::AclFileAuthorizer;
pub use allow_all::AllowAllAuthenticator;
pub use password_file::PasswordFileAuthenticator;
pub use traits::*;

use crate::config::{AclBackend, AclConfig, AuthBackend, AuthConfig};
use std::sync::Arc;

/// Create authenticator based on auth configuration
pub fn new(config: &AuthConfig) -> Result<Arc<dyn Authenticator>, AuthError> {
    match config.backend {
        AuthBackend::AllowAll => Ok(Arc::new(AllowAllAuthenticator)),
        AuthBackend::PasswordFile => Ok(Arc::new(PasswordFileAuthenticator::new(
            &config.password_file,
        )?)),
    }
}

/// Create authorizer based on ACL configuration
pub fn new_authorizer(config: &AclConfig) -> Result<Arc<dyn Authorizer>, AuthError> {
    match config.backend {
        AclBackend::AllowAll => Ok(Arc::new(AllowAllAuthenticator)),
        AclBackend::AclFile => Ok(Arc::new(AclFileAuthorizer::new(&config.acl_file)?)),
    }
}
