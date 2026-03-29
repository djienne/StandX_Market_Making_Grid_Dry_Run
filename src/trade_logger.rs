//! Buffered per-slot CSV trade logger.
//!
//! Hot path (log_fill) appends to an in-memory buffer.
//! flush() writes buffered rows to disk. Called periodically and on shutdown.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use chrono::Utc;

pub struct TradeRecord {
    pub timestamp: String,
    pub symbol: String,
    pub side: String,
    pub price: f64,
    pub size: f64,
    pub level: usize,
    pub position_after: f64,
    pub realized_pnl: f64,
    pub available_capital: f64,
    pub portfolio_value: f64,
}

pub struct TradeLogger {
    path: PathBuf,
    buffer: Vec<TradeRecord>,
    header_written: bool,
    symbol: String,
}

impl TradeLogger {
    pub fn new(path: PathBuf, symbol: &str) -> Self {
        let header_written = path.exists();
        Self {
            path,
            buffer: Vec::new(),
            header_written,
            symbol: symbol.to_string(),
        }
    }

    /// O(1) buffer append - no disk I/O.
    pub fn log_fill(
        &mut self,
        side: &str,
        price: f64,
        size: f64,
        level: usize,
        position_after: f64,
        realized_pnl: f64,
        available_capital: f64,
        portfolio_value: f64,
    ) {
        self.buffer.push(TradeRecord {
            timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            symbol: self.symbol.clone(),
            side: side.to_string(),
            price,
            size,
            level,
            position_after,
            realized_pnl,
            available_capital,
            portfolio_value,
        });
    }

    /// Write buffered rows to CSV file.
    pub fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let mut file = match OpenOptions::new().create(true).append(true).open(&self.path) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("Failed to open trade log {}: {}", self.path.display(), e);
                return;
            }
        };

        if !self.header_written {
            let _ = writeln!(
                file,
                "timestamp,symbol,side,price,size,level,position_after,realized_pnl,available_capital,portfolio_value"
            );
            self.header_written = true;
        }

        for record in &self.buffer {
            let _ = writeln!(
                file,
                "{},{},{},{:.8},{:.8},{},{:.8},{:.8},{:.4},{:.4}",
                record.timestamp,
                record.symbol,
                record.side,
                record.price,
                record.size,
                record.level,
                record.position_after,
                record.realized_pnl,
                record.available_capital,
                record.portfolio_value,
            );
        }

        self.buffer.clear();
    }

    pub fn pending_count(&self) -> usize {
        self.buffer.len()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
