use crate::session::Session;

pub struct Router {
}

impl Router {
    pub fn new() -> Self {
        Self {
        }
    }

    pub async fn unsubscribe_all(&self, _session: &Session) {
    }
}
