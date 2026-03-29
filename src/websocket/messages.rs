//! StandX WebSocket message parsing.
//! Copied from standx with crate path adjustments.

use serde::Deserialize;
use chrono::DateTime;
use thiserror::Error;

use crate::types::{OrderbookSnapshot, Symbol};

#[derive(Error, Debug)]
pub enum MessageError {
    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Invalid price format: {0}")]
    InvalidPrice(String),

    #[error("Invalid quantity format: {0}")]
    InvalidQuantity(String),

    #[error("Invalid timestamp format: {0}")]
    InvalidTimestamp(String),

    #[error("Unknown channel: {0}")]
    UnknownChannel(String),

    #[error("Missing required field: {0}")]
    MissingField(String),
}

#[derive(Debug, Deserialize)]
pub struct RawMessage {
    #[allow(dead_code)]
    pub seq: Option<u64>,
    pub channel: Option<String>,
    pub data: Option<serde_json::Value>,
    pub code: Option<i32>,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DepthBookData {
    pub symbol: String,
    pub asks: Vec<(String, String)>,
    pub bids: Vec<(String, String)>,
    pub sequence: Option<u64>,
    #[serde(default)]
    pub time: Option<serde_json::Value>,
    #[serde(default)]
    pub last_price: Option<String>,
    #[serde(default)]
    pub mark_price: Option<String>,
}

#[derive(Debug)]
pub enum StandXMessage {
    DepthBook(DepthBookData),
    Auth { code: i32, message: String },
    Error { code: i32, message: String },
    Unknown(serde_json::Value),
}

impl StandXMessage {
    pub fn parse(data: &[u8]) -> Result<Self, MessageError> {
        let raw: RawMessage = serde_json::from_slice(data)?;

        match raw.channel.as_deref() {
            Some("depth_book") => {
                let data = raw.data.ok_or(MessageError::MissingField("data".into()))?;
                let depth: DepthBookData = serde_json::from_value(data)?;
                Ok(StandXMessage::DepthBook(depth))
            }
            Some("auth") => {
                let data = raw.data.ok_or(MessageError::MissingField("data".into()))?;
                let code = data.get("code").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let message = data.get("msg").and_then(|v| v.as_str()).unwrap_or("").to_string();
                Ok(StandXMessage::Auth { code, message })
            }
            Some(_channel) => {
                if let Some(data) = raw.data {
                    Ok(StandXMessage::Unknown(data))
                } else {
                    let value = serde_json::from_slice(data)?;
                    Ok(StandXMessage::Unknown(value))
                }
            }
            None => {
                if let Some(code) = raw.code {
                    let message = raw.message.unwrap_or_default();
                    if code != 0 {
                        Ok(StandXMessage::Error { code, message })
                    } else if let Some(data) = raw.data {
                        Ok(StandXMessage::Unknown(data))
                    } else {
                        let value = serde_json::from_slice(data)?;
                        Ok(StandXMessage::Unknown(value))
                    }
                } else if let Some(data) = raw.data {
                    Ok(StandXMessage::Unknown(data))
                } else {
                    let value = serde_json::from_slice(data)?;
                    Ok(StandXMessage::Unknown(value))
                }
            }
        }
    }

    pub fn parse_str(s: &str) -> Result<Self, MessageError> {
        Self::parse(s.as_bytes())
    }
}

impl DepthBookData {
    #[inline]
    pub fn to_snapshot(&self, max_levels: usize, received_at_ns: i64) -> Result<OrderbookSnapshot, MessageError> {
        let mut snapshot = OrderbookSnapshot::new(Symbol::new(&self.symbol));
        snapshot.set_bids_from_strings(&self.bids, max_levels);
        snapshot.set_asks_from_strings(&self.asks, max_levels);
        snapshot.sequence = self.sequence.unwrap_or(0);
        if let Some(ref time_val) = self.time {
            snapshot.timestamp_ns = parse_timestamp_value(time_val)?;
        }
        snapshot.received_at_ns = received_at_ns;
        Ok(snapshot)
    }
}

pub fn parse_timestamp_value(val: &serde_json::Value) -> Result<i64, MessageError> {
    match val {
        serde_json::Value::Number(n) => {
            let ms = n.as_i64().unwrap_or(0);
            Ok(ms.saturating_mul(1_000_000))
        }
        serde_json::Value::String(s) => parse_timestamp(s),
        _ => Ok(0),
    }
}

#[inline]
pub fn parse_timestamp(s: &str) -> Result<i64, MessageError> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp_nanos_opt().unwrap_or(0));
    }
    if !s.ends_with('Z') && s.len() < 64 {
        let mut buf = [0u8; 64];
        let bytes = s.as_bytes();
        buf[..bytes.len()].copy_from_slice(bytes);
        buf[bytes.len()] = b'Z';
        if let Ok(s_with_z) = std::str::from_utf8(&buf[..bytes.len() + 1]) {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s_with_z) {
                return Ok(dt.timestamp_nanos_opt().unwrap_or(0));
            }
        }
    }
    Err(MessageError::InvalidTimestamp(s.to_string()))
}

pub fn current_time_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

pub fn subscribe_message(channel: &str, symbol: &str) -> String {
    serde_json::json!({
        "subscribe": { "channel": channel, "symbol": symbol }
    }).to_string()
}
