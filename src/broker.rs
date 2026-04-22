use crate::auth::{Authenticator, Authorizer};
use crate::config::Config;
use crate::protocol::packet::{PublishPacket, QoS};
use crate::router::Router;
use crate::session::{Mailbox, Session, SessionFuture, TakeoverAction};
use crate::storage::Storage;
use crate::transport::AsyncStream;
use dashmap::{mapref::entry::Entry, DashMap};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

pub struct Broker {
    sessions: Mutex<HashMap<String, Session>>,
    named_clients: DashMap<String, TakeoverAction>,
    router: Router,
    config: Config,
    storage: Arc<dyn Storage>,
    authenticator: Arc<dyn Authenticator>,
    authorizer: Arc<dyn Authorizer>,
}

impl Broker {
    pub fn new(
        config: Config,
        storage: Arc<dyn Storage>,
        authenticator: Arc<dyn Authenticator>,
        authorizer: Arc<dyn Authorizer>,
    ) -> Self {
        let retained_message_limit = config.retained_message_limit;
        Self {
            sessions: Mutex::new(HashMap::new()),
            named_clients: DashMap::new(),
            router: Router::new(retained_message_limit, Arc::clone(&storage)),
            config,
            storage,
            authenticator,
            authorizer,
        }
    }

    pub async fn add_client(&self, stream: Box<dyn AsyncStream>) -> SessionFuture<'_> {
        let (handle, future) = Session::new(self, stream);
        let mut sessions = self.sessions.lock().await;
        info!("Added session {} to broker", handle.id());
        sessions.insert(handle.id().to_owned(), handle);
        future
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn storage(&self) -> &Arc<dyn Storage> {
        &self.storage
    }

    pub fn authenticator(&self) -> &Arc<dyn Authenticator> {
        &self.authenticator
    }

    pub fn authorizer(&self) -> &Arc<dyn Authorizer> {
        &self.authorizer
    }

    /// Send Cancel to all sessions and clear broker state.
    /// Caller is responsible for awaiting session futures to completion.
    pub async fn cancel_all_sessions(&self) {
        let sessions: Vec<Session> = {
            let mut locked = self.sessions.lock().await;
            locked.drain().map(|(_, s)| s).collect()
        };

        info!("Cancelling {} sessions", sessions.len());

        for session in &sessions {
            session.cancel().await;
        }

        self.named_clients.clear();
        info!("All sessions cancelled");
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
