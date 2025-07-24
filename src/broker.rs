use crate::protocol::packet::QoS;
use crate::session::{Session, Mailbox};
use crate::router::Router;
use crate::transport::AsyncStream;
use dashmap::{DashMap, mapref::entry::Entry};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use tracing::{info, error};

pub struct Broker {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    named_clients: DashMap<String, String>,
    router: Router,
}

impl Broker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            named_clients: DashMap::new(),
            router: Router::new(),
        })
    }

    pub async fn add_client(self: &Arc<Self>, stream: Box<dyn AsyncStream>) {
        // Lock the sessions
        let mut sessions = self.sessions.lock().await;

        let session = Session::spawn(
            Arc::clone(self),
            stream,
        ).await;

        info!("Added session {} to broker", session.id());
        sessions.insert(session.id().to_owned(), session);
    }

    pub async fn clean_all_sessions(&self) {
        // Lock the sessions 
        let mut sessions = self.sessions.lock().await;

        self.named_clients.clear();
        let old_sessions: Vec<(String, Arc<Session>)> = sessions.drain().collect();

        drop(sessions);
        // Unlock the sessions

        info!("Begin clear sessions");

        let mut handles = Vec::new();
        handles.reserve(old_sessions.len());
        for (_, session) in old_sessions {
            session.cancel().await.map(|h| handles.push(h));
        }
        for handle in handles {
            handle.await.unwrap_or_else(|e| {
                error!("Error in join task: {}", e);
            });
        }

        info!("All sessions cleaned");
    }

    pub async fn resolve_collision(&self, client_id: &str, session_id: &str) -> Option<Arc<Session>> {
        match self.named_clients.entry(client_id.to_owned()) {
            Entry::Occupied(entry) => {
                match self.sessions.lock().await.get(&entry.get().clone()) {
                    Some(session) => Some(Arc::clone(session)),
                    None => None
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(session_id.to_owned());
                None
            }
        }
    }

    pub async fn remove_session(&self, session: &Session) {
        // Lock the sessions
        let mut sessions = self.sessions.lock().await;

        self.router.unsubscribe_all(session.id()).await;
        if let Some(ref client_id) = session.client_id().await {
            self.named_clients.remove(client_id);
        }
        sessions.remove(session.id());

        info!("Removed session {} from broker", session.id());
    }

    pub async fn subscribe(&self, session_id: &str, sender: Mailbox, topic_filters: &Vec<(String, QoS)>) -> Vec<u8> {
        self.router.subscribe(session_id, sender, topic_filters).await
    }

    pub async fn unsubscribe(&self, session_id: &str, topic_filters: &Vec<String>) {
        self.router.unsubscribe(session_id, topic_filters).await
    }

    pub async fn unsubscribe_all(&self, session_id: &str) {
        self.router.unsubscribe_all(session_id).await
    }

    pub async fn route(&self, topic: &str, payload: &[u8]) {
        self.router.route(topic, payload).await
    }
}

