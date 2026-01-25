use crate::config::Config;
use crate::protocol::packet::{PublishPacket, QoS};
use crate::router::Router;
use crate::session::{Mailbox, Session, TakeoverAction};
use crate::storage::Storage;
use crate::transport::AsyncStream;
use dashmap::{mapref::entry::Entry, DashMap};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

pub struct Broker {
    sessions: Mutex<HashMap<String, Session>>,
    named_clients: DashMap<String, TakeoverAction>,
    router: Router,
    config: Config,
    #[allow(dead_code)] // Will be used for session persistence
    storage: Arc<dyn Storage>,
}

impl Broker {
    pub fn new(config: Config, storage: Arc<dyn Storage>) -> Arc<Self> {
        let retained_message_limit = config.retained_message_limit;
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            named_clients: DashMap::new(),
            router: Router::new(retained_message_limit),
            config,
            storage,
        })
    }

    pub async fn add_client(self: &Arc<Self>, stream: Box<dyn AsyncStream>) {
        // Lock the sessions
        let mut sessions = self.sessions.lock().await;

        let session = Session::spawn(Arc::clone(self), stream).await;

        info!("Added session {} to broker", session.id());
        sessions.insert(session.id().to_owned(), session);
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    #[allow(dead_code)] // Will be used for session persistence
    pub fn storage(&self) -> &Arc<dyn Storage> {
        &self.storage
    }

    pub async fn clean_all_sessions(&self) {
        // Lock the sessions
        let mut sessions = self.sessions.lock().await;

        self.named_clients.clear();
        let old_sessions: Vec<(String, Session)> = sessions.drain().collect();

        drop(sessions);
        // Unlock the sessions

        info!("Begin clear sessions");

        let mut handles = Vec::with_capacity(old_sessions.len());
        for (_, session) in old_sessions {
            handles.push(session.cancel().await);
        }
        for handle in handles {
            handle.await.unwrap_or_else(|e| {
                error!("Error in join task: {}", e);
            });
        }

        info!("All sessions cleaned");
    }

    pub async fn has_collision(
        &self,
        client_id: &str,
        action: TakeoverAction,
    ) -> Option<TakeoverAction> {
        match self.named_clients.entry(client_id.to_owned()) {
            Entry::Occupied(entry) => Some(entry.get().clone()),
            Entry::Vacant(entry) => {
                entry.insert(action);
                None
            }
        }
    }

    pub async fn remove_session(&self, session_id: impl AsRef<str>, client_id: Option<&String>) {
        // Lock the sessions
        let mut sessions = self.sessions.lock().await;

        self.router.unsubscribe_all(session_id.as_ref()).await;
        sessions.remove(session_id.as_ref());
        client_id.and_then(|cid| self.named_clients.remove(cid));

        info!("Removed session {} from broker", session_id.as_ref());
    }

    pub async fn subscribe(
        &self,
        session_id: &str,
        sender: Mailbox,
        topic_filters: &[(String, QoS)],
    ) -> (Vec<u8>, Vec<PublishPacket>) {
        self.router
            .subscribe(session_id, sender, topic_filters)
            .await
    }

    pub async fn unsubscribe(&self, session_id: &str, topic_filters: &[String]) {
        self.router.unsubscribe(session_id, topic_filters).await
    }

    pub async fn unsubscribe_all(&self, session_id: &str) {
        self.router.unsubscribe_all(session_id).await
    }

    pub async fn route(&self, packet: PublishPacket) {
        self.router.route(packet).await
    }

    pub async fn get_subscriptions(&self, session_id: &str) -> Vec<(String, QoS)> {
        self.router.get_subscriptions(session_id).await
    }
}
