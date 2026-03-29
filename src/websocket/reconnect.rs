//! Shared WebSocket reconnection infrastructure.
//! Copied verbatim from standx.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    pub initial_delay_secs: u64,
    pub max_delay_secs: u64,
    pub stale_timeout_secs: u64,
    pub connect_timeout_secs: u64,
    pub max_retries: Option<u32>,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay_secs: 5,
            max_delay_secs: 60,
            stale_timeout_secs: 60,
            connect_timeout_secs: 30,
            max_retries: None,
        }
    }
}

#[derive(Debug)]
pub struct ReconnectState {
    current_delay: u64,
    reconnect_count: AtomicU64,
    consecutive_failures: u32,
}

impl ReconnectState {
    pub fn new(config: &ReconnectConfig) -> Self {
        Self {
            current_delay: config.initial_delay_secs,
            reconnect_count: AtomicU64::new(0),
            consecutive_failures: 0,
        }
    }

    pub fn reset(&mut self, config: &ReconnectConfig) {
        self.current_delay = config.initial_delay_secs;
        self.consecutive_failures = 0;
    }

    pub fn next_delay(&mut self, config: &ReconnectConfig) -> Option<u64> {
        self.consecutive_failures += 1;
        self.reconnect_count.fetch_add(1, Ordering::Relaxed);
        if let Some(max) = config.max_retries {
            if self.consecutive_failures > max { return None; }
        }
        let delay = self.current_delay;
        self.current_delay = (self.current_delay * 2).min(config.max_delay_secs);
        Some(delay)
    }

    pub fn reconnect_count(&self) -> u64 {
        self.reconnect_count.load(Ordering::Relaxed)
    }
}
