//! WebSocket client with automatic reconnection.
//! Copied from standx with crate path adjustments.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout, Instant};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, Error as WsError},
};
use tracing::{debug, error, info, warn};

use crate::config::WebSocketConfig;
use super::messages::{StandXMessage, subscribe_message, current_time_ns};
use super::reconnect::{ReconnectConfig, ReconnectState};

#[derive(Debug)]
pub enum WsEvent {
    Connected,
    Disconnected(String),
    Message(StandXMessage, i64),
    ParseError(String),
    Error(String),
}

#[derive(Debug, Default)]
pub struct WsStats {
    pub messages_received: AtomicU64,
    pub reconnect_count: AtomicU64,
    pub bytes_received: AtomicU64,
    pub last_message_ns: AtomicU64,
}

pub struct WsClient {
    url: String,
    reconnect_config: ReconnectConfig,
    symbols: Vec<String>,
    running: Arc<AtomicBool>,
    stats: Arc<WsStats>,
}

impl WsClient {
    pub fn new(config: WebSocketConfig, symbols: Vec<String>) -> Self {
        Self {
            url: config.url.clone(),
            reconnect_config: config.to_reconnect_config(),
            symbols,
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(WsStats::default()),
        }
    }

    pub fn stats(&self) -> Arc<WsStats> { Arc::clone(&self.stats) }
    pub fn is_running(&self) -> bool { self.running.load(Ordering::Acquire) }
    pub fn stop(&self) { self.running.store(false, Ordering::Release); }

    pub async fn run(self: Arc<Self>) -> mpsc::Receiver<WsEvent> {
        let (tx, rx) = mpsc::channel(10000);
        let client = Arc::clone(&self);
        self.running.store(true, Ordering::Release);
        tokio::spawn(async move { client.connection_loop(tx).await; });
        rx
    }

    async fn connection_loop(&self, tx: mpsc::Sender<WsEvent>) {
        let mut reconnect_state = ReconnectState::new(&self.reconnect_config);
        while self.running.load(Ordering::Acquire) {
            info!("Connecting to {}", self.url);
            match self.connect_and_run(&tx).await {
                Ok(_) => {
                    info!("Connection closed gracefully");
                    reconnect_state.reset(&self.reconnect_config);
                }
                Err(e) => {
                    error!("Connection error: {}", e);
                    let _ = tx.send(WsEvent::Error(e.to_string())).await;
                }
            }
            if !self.running.load(Ordering::Acquire) { break; }
            match reconnect_state.next_delay(&self.reconnect_config) {
                Some(delay) => {
                    self.stats.reconnect_count.fetch_add(1, Ordering::Relaxed);
                    let _ = tx.send(WsEvent::Disconnected(format!("Reconnecting in {}s", delay))).await;
                    warn!("Reconnecting in {}s (attempt #{})", delay, reconnect_state.reconnect_count());
                    sleep(Duration::from_secs(delay)).await;
                }
                None => {
                    error!("Max retries exceeded, stopping");
                    break;
                }
            }
        }
        info!("WebSocket client stopped");
    }

    async fn connect_and_run(&self, tx: &mpsc::Sender<WsEvent>) -> Result<(), WsError> {
        let connect_timeout_dur = Duration::from_secs(self.reconnect_config.connect_timeout_secs);
        let (ws_stream, _) = timeout(connect_timeout_dur, connect_async(&self.url))
            .await
            .map_err(|_| WsError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, "Connection timeout")))??;

        info!("Connected to {}", self.url);
        let _ = tx.send(WsEvent::Connected).await;

        let (mut write, mut read) = ws_stream.split();
        for symbol in &self.symbols {
            let sub_msg = subscribe_message("depth_book", symbol);
            debug!("Subscribing to depth_book for {}", symbol);
            write.send(Message::Text(sub_msg)).await?;
        }

        let stale_timeout = Duration::from_secs(self.reconnect_config.stale_timeout_secs);
        let mut last_message = Instant::now();

        loop {
            if !self.running.load(Ordering::Acquire) { break; }
            if last_message.elapsed() > stale_timeout {
                warn!("Connection stale (no message for {:?}), reconnecting", stale_timeout);
                break;
            }
            match timeout(stale_timeout, read.next()).await {
                Ok(Some(Ok(msg))) => {
                    last_message = Instant::now();
                    self.handle_message(msg, tx).await;
                }
                Ok(Some(Err(e))) => {
                    error!("WebSocket error: {}", e);
                    return Err(e);
                }
                Ok(None) => { info!("WebSocket stream ended"); break; }
                Err(_) => continue,
            }
        }
        Ok(())
    }

    async fn handle_message(&self, msg: Message, tx: &mpsc::Sender<WsEvent>) {
        let received_at = current_time_ns();
        match msg {
            Message::Text(text) => {
                self.stats.bytes_received.fetch_add(text.len() as u64, Ordering::Relaxed);
                self.stats.messages_received.fetch_add(1, Ordering::Relaxed);
                self.stats.last_message_ns.store(received_at as u64, Ordering::Relaxed);
                match StandXMessage::parse_str(&text) {
                    Ok(parsed) => { let _ = tx.send(WsEvent::Message(parsed, received_at)).await; }
                    Err(e) => { debug!("Failed to parse message: {}", e); let _ = tx.send(WsEvent::ParseError(e.to_string())).await; }
                }
            }
            Message::Binary(data) => {
                self.stats.bytes_received.fetch_add(data.len() as u64, Ordering::Relaxed);
                self.stats.messages_received.fetch_add(1, Ordering::Relaxed);
                self.stats.last_message_ns.store(received_at as u64, Ordering::Relaxed);
                match StandXMessage::parse(&data) {
                    Ok(parsed) => { let _ = tx.send(WsEvent::Message(parsed, received_at)).await; }
                    Err(e) => { debug!("Failed to parse binary: {}", e); let _ = tx.send(WsEvent::ParseError(e.to_string())).await; }
                }
            }
            Message::Ping(_) | Message::Pong(_) | Message::Close(_) | Message::Frame(_) => {}
        }
    }
}

pub struct WsClientBuilder {
    url: String,
    reconnect_config: ReconnectConfig,
    symbols: Vec<String>,
}

impl WsClientBuilder {
    pub fn new() -> Self {
        let default_config = WebSocketConfig::default();
        Self {
            url: default_config.url.clone(),
            reconnect_config: default_config.to_reconnect_config(),
            symbols: vec!["BTC-USD".to_string()],
        }
    }

    pub fn symbols(mut self, symbols: Vec<String>) -> Self { self.symbols = symbols; self }
    pub fn config(mut self, config: WebSocketConfig) -> Self {
        self.url = config.url.clone();
        self.reconnect_config = config.to_reconnect_config();
        self
    }
    pub fn build(self) -> Arc<WsClient> {
        Arc::new(WsClient {
            url: self.url,
            reconnect_config: self.reconnect_config,
            symbols: self.symbols,
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(WsStats::default()),
        })
    }
}

impl Default for WsClientBuilder {
    fn default() -> Self { Self::new() }
}
