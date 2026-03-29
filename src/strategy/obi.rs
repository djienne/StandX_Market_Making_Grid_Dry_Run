//! OBI (Order Book Imbalance) market making strategy.
//!
//! Adapted from standx: removed SharedEquity, SharedSymbolInfo, SharedAlpha.
//! Tick/lot size and sizing are set directly via setters. Core quote math is identical.

use tracing::{debug, info};
use crate::config::StrategyConfig;
use crate::types::OrderbookSnapshot;
use super::rolling::RollingStats;
use super::quotes::{Quote, MAX_ORDER_LEVELS};

const MIN_SAMPLES_FOR_QUOTE: usize = 100;

pub struct ObiStrategy {
    config: StrategyConfig,
    mid_price_chg_stats: RollingStats,
    imbalance_stats: RollingStats,
    prev_mid_price: Option<f64>,
    position: f64,
    step_count: u64,
    last_update_step: u64,
    volatility: f64,
    alpha: f64,
    warmed_up: bool,
    first_timestamp_ns: Option<i64>,
    latest_timestamp_ns: i64,
    required_history_ns: u64,
    total_samples: usize,
    logged_valid_for_trading: bool,
    // Direct values (set by grid runner, no shared atomics)
    order_qty_dollar: f64,
    max_position_dollar: f64,
}

impl ObiStrategy {
    pub fn new(config: StrategyConfig, required_history_secs: f64) -> Self {
        let window_steps = config.window_steps;
        Self {
            config,
            mid_price_chg_stats: RollingStats::new(window_steps),
            imbalance_stats: RollingStats::new(window_steps),
            prev_mid_price: None,
            position: 0.0,
            step_count: 0,
            last_update_step: 0,
            volatility: 0.0,
            alpha: 0.0,
            warmed_up: false,
            first_timestamp_ns: None,
            latest_timestamp_ns: 0,
            required_history_ns: (required_history_secs * 1_000_000_000.0) as u64,
            total_samples: 0,
            logged_valid_for_trading: false,
            order_qty_dollar: 0.0,
            max_position_dollar: f64::MAX,
        }
    }

    pub fn set_position(&mut self, position: f64) { self.position = position; }
    pub fn set_order_qty_dollar(&mut self, v: f64) { self.order_qty_dollar = v; }
    pub fn set_max_position_dollar(&mut self, v: f64) { self.max_position_dollar = v; }
    pub fn set_tick_size(&mut self, v: f64) { self.config.tick_size = v; }
    pub fn set_lot_size(&mut self, v: f64) { self.config.lot_size = v; }

    #[inline] pub fn position(&self) -> f64 { self.position }
    #[inline] pub fn is_warmed_up(&self) -> bool { self.warmed_up }
    #[inline] pub fn volatility(&self) -> f64 { self.volatility }
    #[inline] pub fn alpha(&self) -> f64 { self.alpha }
    #[inline] pub fn config(&self) -> &StrategyConfig { &self.config }

    #[inline]
    pub fn is_valid_for_trading(&self) -> bool {
        self.warmed_up && self.history_duration_ns() >= self.required_history_ns
    }

    #[inline]
    pub fn history_duration_ns(&self) -> u64 {
        match self.first_timestamp_ns {
            Some(first) if self.latest_timestamp_ns > first => (self.latest_timestamp_ns - first) as u64,
            _ => 0,
        }
    }

    #[inline]
    pub fn history_duration_secs(&self) -> f64 {
        self.history_duration_ns() as f64 / 1_000_000_000.0
    }

    pub fn update(&mut self, snapshot: &OrderbookSnapshot) -> Option<Quote> {
        let timestamp = snapshot.timestamp_ns;
        if timestamp > 0 {
            if self.first_timestamp_ns.is_none() {
                self.first_timestamp_ns = Some(timestamp);
            }
            self.latest_timestamp_ns = timestamp;
        }

        let mid_price = snapshot.mid_price()?;

        if let Some(prev_mid) = self.prev_mid_price {
            let mid_chg = mid_price - prev_mid;
            self.mid_price_chg_stats.push(mid_chg);
            self.total_samples += 1;
        }
        self.prev_mid_price = Some(mid_price);

        let imbalance = self.calculate_imbalance(snapshot, mid_price);
        self.imbalance_stats.push(imbalance);

        self.step_count += 1;
        let steps_since = self.step_count - self.last_update_step;
        if steps_since < self.config.update_interval_steps as u64 {
            return None;
        }
        self.last_update_step = self.step_count;

        if self.total_samples < MIN_SAMPLES_FOR_QUOTE {
            return None;
        }

        if !self.warmed_up {
            debug!("[{}] Strategy warmed up: {} samples", snapshot.symbol, self.total_samples);
        }
        self.warmed_up = true;

        let vol_raw = self.mid_price_chg_stats.std();
        self.volatility = vol_raw * self.config.vol_scale();
        self.alpha = self.imbalance_stats.zscore(imbalance);

        self.calculate_quote(snapshot, mid_price)
    }

    #[inline]
    fn calculate_imbalance(&self, snapshot: &OrderbookSnapshot, mid_price: f64) -> f64 {
        let depth_pct = self.config.looking_depth;
        let lower_bound = mid_price * (1.0 - depth_pct);
        let upper_bound = mid_price * (1.0 + depth_pct);

        let mut sum_bid_qty = 0.0;
        for level in &snapshot.bids[..snapshot.bid_count as usize] {
            if level.price < lower_bound { break; }
            sum_bid_qty += level.quantity;
        }

        let mut sum_ask_qty = 0.0;
        for level in &snapshot.asks[..snapshot.ask_count as usize] {
            if level.price > upper_bound { break; }
            sum_ask_qty += level.quantity;
        }

        sum_bid_qty - sum_ask_qty
    }

    #[inline]
    fn calculate_quote(&mut self, snapshot: &OrderbookSnapshot, mid_price: f64) -> Option<Quote> {
        let best_bid = snapshot.best_bid_price()?;
        let best_ask = snapshot.best_ask_price()?;

        let tick_size = self.config.tick_size;
        let base_half_spread_tick = if self.config.vol_to_half_spread > 0.0 && self.volatility > 0.0 {
            (self.volatility * self.config.vol_to_half_spread) / tick_size
        } else if self.config.half_spread_bps > 0.0 {
            mid_price * (self.config.half_spread_bps / 10000.0) / tick_size
        } else if self.config.half_spread > 0.0 {
            self.config.half_spread / tick_size
        } else {
            1.0
        };

        let fair_price = mid_price + self.config.c1() * self.alpha;

        let max_position_dollar = if self.max_position_dollar > 0.0 { self.max_position_dollar } else { f64::MAX };
        let normalized_position = (self.position * mid_price) / max_position_dollar;
        let clamped_position = normalized_position.clamp(-1.0, 1.0);

        let num_levels = self.config.order_levels.min(MAX_ORDER_LEVELS);
        let multipliers = [1.0, self.config.spread_level_multiplier];

        if self.order_qty_dollar <= 0.0 {
            return None;
        }

        let mut bid_prices = [0.0; MAX_ORDER_LEVELS];
        let mut ask_prices = [0.0; MAX_ORDER_LEVELS];
        let mut bid_floored = [false; MAX_ORDER_LEVELS];
        let mut ask_floored = [false; MAX_ORDER_LEVELS];

        for level in 0..num_levels {
            let half_spread_tick = base_half_spread_tick * multipliers[level];
            let bid_depth_tick = (half_spread_tick * (1.0 + self.config.skew * clamped_position)).max(0.0);
            let ask_depth_tick = (half_spread_tick * (1.0 - self.config.skew * clamped_position)).max(0.0);

            let raw_bid = fair_price - bid_depth_tick * tick_size;
            let raw_ask = fair_price + ask_depth_tick * tick_size;

            let clamped_bid = raw_bid.min(best_bid);
            let clamped_ask = raw_ask.max(best_ask);

            let (floored_bid, bf) = if self.config.min_half_spread_bps > 0.0 {
                let level_min_bps = self.config.min_half_spread_bps * multipliers[level];
                let min_bid = mid_price * (1.0 - level_min_bps / 10000.0);
                if clamped_bid > min_bid { (min_bid, true) } else { (clamped_bid, false) }
            } else { (clamped_bid, false) };

            let (floored_ask, af) = if self.config.min_half_spread_bps > 0.0 {
                let level_min_bps = self.config.min_half_spread_bps * multipliers[level];
                let min_ask = mid_price * (1.0 + level_min_bps / 10000.0);
                if clamped_ask < min_ask { (min_ask, true) } else { (clamped_ask, false) }
            } else { (clamped_ask, false) };

            bid_prices[level] = (floored_bid / tick_size).floor() * tick_size;
            ask_prices[level] = (floored_ask / tick_size).ceil() * tick_size;
            bid_floored[level] = bf;
            ask_floored[level] = af;
        }

        let order_qty_per_level = self.order_qty_dollar / mid_price;
        let lot_size = self.config.lot_size;
        let quantity = ((order_qty_per_level / lot_size).round() * lot_size).max(lot_size);

        let valid_for_trading = self.is_valid_for_trading();
        if valid_for_trading && !self.logged_valid_for_trading {
            self.logged_valid_for_trading = true;
            info!("[{}] Strategy valid for trading (history={:.0}s, samples={})",
                snapshot.symbol, self.history_duration_secs(), self.total_samples);
        }

        Some(Quote {
            symbol: snapshot.symbol,
            bid_prices,
            ask_prices,
            num_levels,
            quantity,
            mid_price,
            spread: ask_prices[0] - bid_prices[0],
            volatility: self.volatility,
            alpha: self.alpha,
            position: self.position,
            half_spread_tick: base_half_spread_tick,
            valid_for_trading,
            history_secs: self.history_duration_secs(),
            bid_floored,
            ask_floored,
        })
    }

    pub fn reset_state(&mut self) {
        self.mid_price_chg_stats.clear();
        self.imbalance_stats.clear();
        self.prev_mid_price = None;
        self.step_count = 0;
        self.last_update_step = 0;
        self.volatility = 0.0;
        self.alpha = 0.0;
        self.warmed_up = false;
        self.first_timestamp_ns = None;
        self.latest_timestamp_ns = 0;
        self.total_samples = 0;
        self.logged_valid_for_trading = false;
    }
}
