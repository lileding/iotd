pub mod memory;
pub mod sqlite;
pub mod traits;
pub mod types;

pub use memory::InMemoryStorage;
pub use sqlite::SqliteStorage;
pub use traits::*;
pub use types::*;

use crate::config::{PersistenceConfig, StorageBackend};
use std::sync::Arc;

/// Create storage based on persistence configuration
pub fn new(config: &PersistenceConfig) -> StorageResult<Arc<dyn Storage>> {
    match config.backend {
        StorageBackend::Memory => Ok(Arc::new(InMemoryStorage::new())),
        StorageBackend::Sqlite => Ok(Arc::new(SqliteStorage::new(&config.database_path)?)),
    }
}
