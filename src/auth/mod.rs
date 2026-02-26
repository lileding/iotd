pub mod allow_all;
pub mod password_file;
pub mod traits;

pub use allow_all::AllowAllAuthenticator;
pub use password_file::PasswordFileAuthenticator;
pub use traits::*;

use crate::config::{AuthBackend, AuthConfig};
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
