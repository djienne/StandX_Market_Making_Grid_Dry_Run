//! Configuration for the grid dry-run simulator.
//!
//! GridConfig is the top-level config loaded from grid_config.json.
//! WebSocketConfig and StrategyConfig are adapted from standx.

use std::collections::HashMap;
use std::path::Path;
use serde::Deserialize;
use thiserror::Error;

use crate::websocket::reconnect::ReconnectConfig;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("Failed to parse config: {0}")]
    ParseError(#[from] serde_json::Error),
    #[error("Invalid configuration: {0}")]
    ValidationError(String),
}

// ── WebSocketConfig (from standx) ──

#[derive(Debug, Clone, Deserialize)]
pub struct WebSocketConfig {
    #[serde(default = "default_ws_url")]
    pub url: String,
    #[serde(default = "default_reconnect_delay")]
    pub reconnect_delay_secs: u64,
    #[serde(default = "default_max_reconnect_delay")]
    pub max_reconnect_delay_secs: u64,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_stale_timeout")]
    pub stale_timeout_secs: u64,
}

fn default_ws_url() -> String { "wss://perps.standx.com/ws-stream/v1".to_string() }
fn default_reconnect_delay() -> u64 { 5 }
fn default_max_reconnect_delay() -> u64 { 60 }
fn default_connect_timeout() -> u64 { 30 }
fn default_stale_timeout() -> u64 { 60 }

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            url: default_ws_url(),
            reconnect_delay_secs: default_reconnect_delay(),
            max_reconnect_delay_secs: default_max_reconnect_delay(),
            connect_timeout_secs: default_connect_timeout(),
            stale_timeout_secs: default_stale_timeout(),
        }
    }
}

impl WebSocketConfig {
    pub fn to_reconnect_config(&self) -> ReconnectConfig {
        ReconnectConfig {
            initial_delay_secs: self.reconnect_delay_secs,
            max_delay_secs: self.max_reconnect_delay_secs,
            stale_timeout_secs: self.stale_timeout_secs,
            connect_timeout_secs: self.connect_timeout_secs,
            max_retries: None,
        }
    }
}

// ── StrategyConfig (from standx, used as defaults for grid) ──

#[derive(Debug, Clone, Deserialize)]
pub struct StrategyDefaults {
    #[serde(default = "default_tick_size")]
    pub tick_size: f64,
    #[serde(default = "default_lot_size")]
    pub lot_size: f64,
    #[serde(default = "default_step_ns")]
    pub step_ns: u64,
    #[serde(default = "default_window_steps")]
    pub window_steps: usize,
    #[serde(default = "default_looking_depth")]
    pub looking_depth: f64,
    #[serde(default = "default_alpha_source")]
    pub alpha_source: String,
    #[serde(default = "default_update_interval_steps")]
    pub update_interval_steps: usize,
}

fn default_tick_size() -> f64 { 0.01 }
fn default_lot_size() -> f64 { 0.001 }
fn default_step_ns() -> u64 { 100_000_000 }
fn default_window_steps() -> usize { 6000 }
fn default_looking_depth() -> f64 { 0.025 }
fn default_alpha_source() -> String { "standx".to_string() }
fn default_update_interval_steps() -> usize { 1 }

impl Default for StrategyDefaults {
    fn default() -> Self {
        Self {
            tick_size: default_tick_size(),
            lot_size: default_lot_size(),
            step_ns: default_step_ns(),
            window_steps: default_window_steps(),
            looking_depth: default_looking_depth(),
            alpha_source: default_alpha_source(),
            update_interval_steps: default_update_interval_steps(),
        }
    }
}

// ── StrategyConfig (full, constructed per-slot from GridParams + defaults) ──

#[derive(Debug, Clone)]
pub struct StrategyConfig {
    pub tick_size: f64,
    pub lot_size: f64,
    pub step_ns: u64,
    pub window_steps: usize,
    pub update_interval_steps: usize,
    pub vol_to_half_spread: f64,
    pub half_spread: f64,
    pub half_spread_bps: f64,
    pub min_half_spread_bps: f64,
    pub skew: f64,
    pub c1: f64,
    pub c1_ticks: f64,
    pub looking_depth: f64,
    pub order_levels: usize,
    pub spread_level_multiplier: f64,
    pub alpha_source: String,
    pub binance_stale_ms: u64,
    pub leverage: f64,
}

impl StrategyConfig {
    #[inline]
    pub fn c1(&self) -> f64 {
        if self.c1 > 0.0 { self.c1 } else { self.c1_ticks * self.tick_size }
    }

    #[inline]
    pub fn vol_scale(&self) -> f64 {
        (1_000_000_000.0 / self.step_ns as f64).sqrt()
    }
}

// ── GridParams (per-slot parameters from Cartesian product) ──

#[derive(Debug, Clone)]
pub struct GridParams {
    pub vol_to_half_spread: f64,
    pub min_half_spread_bps: f64,
    pub skew: f64,
    pub spread_factor_level1: f64,
    pub num_levels: usize,
    pub c1_ticks: f64,
}

impl GridParams {
    /// Deterministic key for state persistence.
    pub fn param_key(&self) -> String {
        format!(
            "v{}_m{}_s{}_f{}_l{}_c{}",
            self.vol_to_half_spread,
            self.min_half_spread_bps,
            self.skew,
            self.spread_factor_level1,
            self.num_levels,
            self.c1_ticks,
        )
    }
}

// ── GridConfig (top-level grid_config.json) ──

#[derive(Debug, Clone, Deserialize)]
pub struct GridConfig {
    pub symbol: String,
    #[serde(default = "default_capital")]
    pub capital: f64,
    #[serde(default = "default_leverage")]
    pub leverage: u32,
    #[serde(default = "default_warmup_seconds")]
    pub warmup_seconds: f64,
    #[serde(default = "default_summary_interval")]
    pub summary_interval_seconds: f64,
    #[serde(default = "default_sim_latency_ms")]
    pub sim_latency_ms: u64,
    #[serde(default = "default_maker_fee_rate")]
    pub maker_fee_rate: f64,
    #[serde(default = "default_orderbook_levels")]
    pub orderbook_levels: usize,
    pub parameters: HashMap<String, Vec<f64>>,
    #[serde(default)]
    pub fixed: HashMap<String, serde_json::Value>,
    #[serde(default = "default_logs_dir")]
    pub logs_dir: String,
    #[serde(default)]
    pub websocket: WebSocketConfig,
    #[serde(default)]
    pub strategy_defaults: StrategyDefaults,
}

fn default_capital() -> f64 { 1000.0 }
fn default_leverage() -> u32 { 1 }
fn default_warmup_seconds() -> f64 { 600.0 }
fn default_summary_interval() -> f64 { 60.0 }
fn default_sim_latency_ms() -> u64 { 50 }
fn default_maker_fee_rate() -> f64 { 0.0001 }
fn default_orderbook_levels() -> usize { 20 }
fn default_logs_dir() -> String { "logs/grid".to_string() }

impl GridConfig {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: GridConfig = serde_json::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.symbol.is_empty() {
            return Err(ConfigError::ValidationError("symbol must not be empty".into()));
        }
        if self.capital <= 0.0 {
            return Err(ConfigError::ValidationError("capital must be positive".into()));
        }
        if self.parameters.is_empty() {
            return Err(ConfigError::ValidationError("parameters must have at least one axis".into()));
        }
        for (name, values) in &self.parameters {
            if values.is_empty() {
                return Err(ConfigError::ValidationError(
                    format!("parameter '{}' must have at least one value", name),
                ));
            }
        }
        // Check total slots
        let total: usize = self.parameters.values().map(|v| v.len()).product();
        if total > 500 {
            return Err(ConfigError::ValidationError(
                format!("too many parameter combinations: {} (max 500)", total),
            ));
        }
        Ok(())
    }

    /// Build the Cartesian product of all parameter axes.
    pub fn build_params(&self) -> Vec<GridParams> {
        let fixed = &self.fixed;
        let get_fixed_f64 = |key: &str, default: f64| -> f64 {
            fixed.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
        };
        let get_fixed_usize = |key: &str, default: usize| -> usize {
            fixed.get(key).and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(default)
        };

        // Collect axis names and values in deterministic order
        let mut axes: Vec<(&str, &Vec<f64>)> = self.parameters.iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        axes.sort_by_key(|(k, _)| *k);

        // Build Cartesian product
        let mut combos: Vec<Vec<(String, f64)>> = vec![vec![]];
        for (name, values) in &axes {
            let mut new_combos = Vec::with_capacity(combos.len() * values.len());
            for combo in &combos {
                for val in *values {
                    let mut new = combo.clone();
                    new.push((name.to_string(), *val));
                    new_combos.push(new);
                }
            }
            combos = new_combos;
        }

        // Convert to GridParams
        combos.into_iter().map(|combo| {
            let mut params = GridParams {
                vol_to_half_spread: get_fixed_f64("vol_to_half_spread", 8.0),
                min_half_spread_bps: get_fixed_f64("min_half_spread_bps", 4.0),
                skew: get_fixed_f64("skew", 1.0),
                spread_factor_level1: get_fixed_f64("spread_factor_level1", 2.0),
                num_levels: get_fixed_usize("num_levels", 2),
                c1_ticks: get_fixed_f64("c1_ticks", 20.0),
            };
            // Override with axis values
            for (name, val) in &combo {
                match name.as_str() {
                    "vol_to_half_spread" => params.vol_to_half_spread = *val,
                    "min_half_spread_bps" => params.min_half_spread_bps = *val,
                    "skew" => params.skew = *val,
                    "spread_factor_level1" => params.spread_factor_level1 = *val,
                    "num_levels" => params.num_levels = *val as usize,
                    "c1_ticks" => params.c1_ticks = *val,
                    _ => {} // unknown params ignored
                }
            }
            params
        }).collect()
    }

    /// Build a StrategyConfig for a specific slot.
    pub fn build_strategy_config(&self, params: &GridParams) -> StrategyConfig {
        let sd = &self.strategy_defaults;
        StrategyConfig {
            tick_size: sd.tick_size,
            lot_size: sd.lot_size,
            step_ns: sd.step_ns,
            window_steps: sd.window_steps,
            update_interval_steps: sd.update_interval_steps,
            vol_to_half_spread: params.vol_to_half_spread,
            half_spread: 0.0,
            half_spread_bps: 0.0,
            min_half_spread_bps: params.min_half_spread_bps,
            skew: params.skew,
            c1: 0.0,
            c1_ticks: params.c1_ticks,
            looking_depth: sd.looking_depth,
            order_levels: params.num_levels,
            spread_level_multiplier: params.spread_factor_level1,
            alpha_source: sd.alpha_source.clone(),
            binance_stale_ms: 5000,
            leverage: self.leverage as f64,
        }
    }
}
