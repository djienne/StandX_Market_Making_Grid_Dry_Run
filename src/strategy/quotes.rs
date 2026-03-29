//! Quote output structure.
//!
//! Adapted from standx - removed log_quote! macro dependency, using tracing directly.

use crate::types::Symbol;

pub const MAX_ORDER_LEVELS: usize = 2;

#[derive(Debug, Clone)]
pub struct Quote {
    pub symbol: Symbol,
    pub bid_prices: [f64; MAX_ORDER_LEVELS],
    pub ask_prices: [f64; MAX_ORDER_LEVELS],
    pub num_levels: usize,
    pub quantity: f64,
    pub mid_price: f64,
    pub spread: f64,
    pub volatility: f64,
    pub alpha: f64,
    pub position: f64,
    pub half_spread_tick: f64,
    pub valid_for_trading: bool,
    pub history_secs: f64,
    pub bid_floored: [bool; MAX_ORDER_LEVELS],
    pub ask_floored: [bool; MAX_ORDER_LEVELS],
}

impl Quote {
    #[inline]
    pub fn bid_price(&self) -> f64 { self.bid_prices[0] }

    #[inline]
    pub fn ask_price(&self) -> f64 { self.ask_prices[0] }

    pub fn spread_bps(&self) -> f64 {
        if self.mid_price == 0.0 { return 0.0; }
        (self.spread / self.mid_price) * 10000.0
    }
}
