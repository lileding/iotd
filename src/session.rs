use crate::transport::AsyncStream;
use tokio::sync::mpsc;
use uuid::Uuid;
use std::sync::atomic::AtomicBool;

pub struct Session {
    pub session_id: String,
    pub client_id: Option<String>,
    pub stream: Box<dyn AsyncStream>,
    pub connected: AtomicBool,
    #[allow(dead_code)]
    pub clean_session: bool,
    #[allow(dead_code)]
    pub keep_alive: u16,
    #[allow(dead_code)]
    pub message_rx: mpsc::Receiver<bytes::Bytes>,
    #[allow(dead_code)]
    pub message_tx: mpsc::Sender<bytes::Bytes>,
}

impl Session {
    pub fn new(stream: Box<dyn AsyncStream>) -> Self {
        let uuid = Uuid::new_v4().to_string();
        let session_id = format!("session_{}", uuid);
        let (message_tx, message_rx) = mpsc::channel(1000);
        
        Self {
            session_id,
            client_id: None,
            stream,
            connected: AtomicBool::new(false),
            clean_session: true,
            keep_alive: 0,
            message_rx,
            message_tx,
        }
    }
}
