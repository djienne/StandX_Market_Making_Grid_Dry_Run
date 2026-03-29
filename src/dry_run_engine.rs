//! Dry-run fill simulation engine.
//!
//! Ported from lighter_MM/dry_run.py DryRunEngine.
//! Simulates order placement, fill detection (delta-fill), and PnL tracking.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::simulated_order::{BatchOp, OrderSide, SimulatedOrder};
use crate::trade_logger::TradeLogger;
use crate::types::OrderbookSnapshot;

#[derive(Debug, Serialize, Deserialize)]
pub struct PersistedState {
    pub available_capital: f64,
    pub portfolio_value: f64,
    pub position: f64,
    pub entry_vwap: f64,
    pub realized_pnl: f64,
    pub fill_count: u64,
    pub total_volume: f64,
    pub initial_capital: f64,
    #[serde(default)]
    pub initial_portfolio_value: f64,
    pub updated_at: String,
}

pub struct DryRunEngine {
    live_orders: HashMap<u64, SimulatedOrder>,
    next_cid: u64,
    next_eid: u64,

    // PnL state (average-cost-basis)
    pub position: f64,
    pub entry_vwap: f64,
    pub realized_pnl: f64,
    pub available_capital: f64,
    pub portfolio_value: f64,
    pub total_volume: f64,
    pub fill_count: u64,
    pub initial_capital: f64,
    pub initial_portfolio_value: f64,

    // Config
    leverage: u32,
    sim_latency: Duration,
    maker_fee_rate: f64,

    // Persistence
    state_path: PathBuf,

    // Trade logger
    pub trade_logger: TradeLogger,
}

impl DryRunEngine {
    pub fn new(
        capital: f64,
        leverage: u32,
        sim_latency_ms: u64,
        maker_fee_rate: f64,
        state_path: PathBuf,
        trade_logger: TradeLogger,
        slot_cid_base: u64,
    ) -> Self {
        Self {
            live_orders: HashMap::new(),
            next_cid: slot_cid_base,
            next_eid: 900_000_000,
            position: 0.0,
            entry_vwap: 0.0,
            realized_pnl: 0.0,
            available_capital: capital,
            portfolio_value: capital,
            total_volume: 0.0,
            fill_count: 0,
            initial_capital: capital,
            initial_portfolio_value: capital,
            leverage,
            sim_latency: Duration::from_millis(sim_latency_ms),
            maker_fee_rate,
            state_path,
            trade_logger,
        }
    }

    /// Try to restore state from a persisted JSON file.
    pub fn try_load_state(&mut self) -> bool {
        if !self.state_path.exists() {
            return false;
        }
        match fs::read_to_string(&self.state_path) {
            Ok(content) => {
                match serde_json::from_str::<PersistedState>(&content) {
                    Ok(state) => {
                        self.available_capital = state.available_capital;
                        self.portfolio_value = state.portfolio_value;
                        self.position = state.position;
                        self.entry_vwap = state.entry_vwap;
                        self.realized_pnl = state.realized_pnl;
                        self.fill_count = state.fill_count;
                        self.total_volume = state.total_volume;
                        self.initial_capital = state.initial_capital;
                        self.initial_portfolio_value = if state.initial_portfolio_value > 0.0 {
                            state.initial_portfolio_value
                        } else {
                            state.initial_capital
                        };
                        info!("Restored state: pos={:.6} pnl=${:.4} fills={}",
                            self.position, self.realized_pnl, self.fill_count);
                        true
                    }
                    Err(e) => { warn!("Failed to parse state: {}", e); false }
                }
            }
            Err(e) => { warn!("Failed to read state: {}", e); false }
        }
    }

    pub fn save_state(&self) {
        let state = PersistedState {
            available_capital: self.available_capital,
            portfolio_value: self.portfolio_value,
            position: self.position,
            entry_vwap: self.entry_vwap,
            realized_pnl: self.realized_pnl,
            fill_count: self.fill_count,
            total_volume: self.total_volume,
            initial_capital: self.initial_capital,
            initial_portfolio_value: self.initial_portfolio_value,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Some(parent) = self.state_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let tmp = self.state_path.with_extension("tmp");
        match serde_json::to_string_pretty(&state) {
            Ok(json) => {
                if fs::write(&tmp, &json).is_ok() {
                    let _ = fs::rename(&tmp, &self.state_path);
                }
            }
            Err(e) => warn!("Failed to serialize state: {}", e),
        }
    }

    fn alloc_cid(&mut self) -> u64 {
        let cid = self.next_cid;
        self.next_cid += 1;
        cid
    }

    fn alloc_eid(&mut self) -> u64 {
        let eid = self.next_eid;
        self.next_eid += 1;
        eid
    }

    /// Capture the qualifying depth from the snapshot for delta-fill tracking.
    fn capture_prev_by_price(snapshot: &OrderbookSnapshot, side: OrderSide, price: f64) -> HashMap<u64, f64> {
        let mut prev = HashMap::new();
        match side {
            OrderSide::Buy => {
                // Buy limit at P fills when asks <= P
                for level in snapshot.ask_levels() {
                    if level.price > price { break; }
                    prev.insert(level.price.to_bits(), level.quantity);
                }
            }
            OrderSide::Sell => {
                // Sell limit at P fills when bids >= P
                for level in snapshot.bid_levels() {
                    if level.price < price { break; }
                    prev.insert(level.price.to_bits(), level.quantity);
                }
            }
        }
        prev
    }

    /// Process a batch of create/cancel operations.
    pub fn process_batch(&mut self, ops: &[BatchOp], snapshot: &OrderbookSnapshot) {
        let now = Instant::now();
        for op in ops {
            match op {
                BatchOp::Create { side, price, size, level } => {
                    // POST_ONLY check at creation: reject if would immediately match
                    let rejected = match side {
                        OrderSide::Buy => {
                            snapshot.best_ask_price().map_or(false, |ask| *price >= ask)
                        }
                        OrderSide::Sell => {
                            snapshot.best_bid_price().map_or(false, |bid| *price <= bid)
                        }
                    };
                    if rejected {
                        debug!("DRY-RUN POST_ONLY reject {} {} @ {:.2}", side, size, price);
                        continue;
                    }

                    let cid = self.alloc_cid();
                    let eid = self.alloc_eid();
                    let prev_by_price = Self::capture_prev_by_price(snapshot, *side, *price);

                    self.live_orders.insert(cid, SimulatedOrder {
                        client_order_id: cid,
                        side: *side,
                        price: *price,
                        size: *size,
                        original_size: *size,
                        level: *level,
                        created_at: now,
                        eligible_at: now + self.sim_latency,
                        pending_cancel_at: None,
                        prev_by_price,
                        arrival_checked: false,
                    });

                    debug!("DRY-RUN CREATE {} L{}: {:.6} @ {:.2} (cid={} eid={})",
                        side, level, size, price, cid, eid);
                }
                BatchOp::Cancel { client_order_id } => {
                    if let Some(order) = self.live_orders.get_mut(client_order_id) {
                        order.pending_cancel_at = Some(now + self.sim_latency);
                        debug!("DRY-RUN CANCEL cid={}", client_order_id);
                    }
                }
            }
        }
    }

    /// Check all live orders against the current orderbook for fills.
    /// Called on every orderbook update.
    pub fn check_fills(&mut self, snapshot: &OrderbookSnapshot) {
        let now = Instant::now();
        let mid_price = match snapshot.mid_price() {
            Some(m) => m,
            None => return,
        };

        // Collect order IDs to process (avoid borrow issues)
        let order_ids: Vec<u64> = self.live_orders.keys().copied().collect();

        // Sort by price priority (most aggressive first) for fair allocation
        let mut sorted_ids = order_ids.clone();
        sorted_ids.sort_by(|a, b| {
            let oa = &self.live_orders[a];
            let ob = &self.live_orders[b];
            match (oa.side, ob.side) {
                (OrderSide::Buy, OrderSide::Buy) => {
                    ob.price.partial_cmp(&oa.price).unwrap_or(std::cmp::Ordering::Equal)
                }
                (OrderSide::Sell, OrderSide::Sell) => {
                    oa.price.partial_cmp(&ob.price).unwrap_or(std::cmp::Ordering::Equal)
                }
                _ => std::cmp::Ordering::Equal,
            }
        });

        // Track liquidity consumed in this tick (single float per side, matching Python)
        let mut consumed_buy: f64 = 0.0;
        let mut consumed_sell: f64 = 0.0;

        let mut fills: Vec<(u64, f64)> = Vec::new();
        let mut to_remove: Vec<u64> = Vec::new();

        for &cid in &sorted_ids {
            let order = match self.live_orders.get_mut(&cid) {
                Some(o) => o,
                None => continue,
            };

            // Skip if not yet eligible (latency sim)
            if now < order.eligible_at {
                // But still check cancel maturity
                if let Some(cancel_at) = order.pending_cancel_at {
                    if now >= cancel_at {
                        to_remove.push(cid);
                    }
                }
                continue;
            }

            // POST_ONLY arrival recheck
            if !order.arrival_checked {
                order.arrival_checked = true;
                let crossed = match order.side {
                    OrderSide::Buy => snapshot.best_ask_price().map_or(false, |ask| order.price >= ask),
                    OrderSide::Sell => snapshot.best_bid_price().map_or(false, |bid| order.price <= bid),
                };
                if crossed {
                    debug!("DRY-RUN POST_ONLY reject at arrival cid={}", cid);
                    to_remove.push(cid);
                    continue;
                }
            }

            // Check for cancel maturity (after fill opportunity per Python)
            // We check fills first, then cancel — but we need to gate cancels
            // that matured before eligible_at already handled above.

            // Delta-fill check
            let fill_size = match order.side {
                OrderSide::Buy => {
                    let best_ask = match snapshot.best_ask_price() {
                        Some(a) => a,
                        None => continue,
                    };
                    if best_ask > order.price {
                        Self::update_prev_by_price_static(
                            &mut order.prev_by_price,
                            snapshot.ask_levels(),
                            order.price,
                            true,
                        );
                        0.0
                    } else {
                        // Sum new liquidity at qualifying prices
                        let mut new_liq = 0.0_f64;
                        for level in snapshot.ask_levels() {
                            if level.price > order.price { break; }
                            let price_key = level.price.to_bits();
                            let prev_qty = order.prev_by_price.get(&price_key).copied().unwrap_or(0.0);
                            let delta = (level.quantity - prev_qty).max(0.0);
                            new_liq += delta;
                        }
                        // Subtract total consumed by earlier buy orders this tick
                        new_liq -= consumed_buy;
                        // Update prev_by_price for next tick
                        Self::update_prev_by_price_static(
                            &mut order.prev_by_price,
                            snapshot.ask_levels(),
                            order.price,
                            true,
                        );
                        if new_liq <= 0.0 {
                            0.0
                        } else {
                            let fill = new_liq.min(order.size);
                            consumed_buy += fill;
                            fill
                        }
                    }
                }
                OrderSide::Sell => {
                    let best_bid = match snapshot.best_bid_price() {
                        Some(b) => b,
                        None => continue,
                    };
                    if best_bid < order.price {
                        Self::update_prev_by_price_static(
                            &mut order.prev_by_price,
                            snapshot.bid_levels(),
                            order.price,
                            false,
                        );
                        0.0
                    } else {
                        let mut new_liq = 0.0_f64;
                        for level in snapshot.bid_levels() {
                            if level.price < order.price { break; }
                            let price_key = level.price.to_bits();
                            let prev_qty = order.prev_by_price.get(&price_key).copied().unwrap_or(0.0);
                            let delta = (level.quantity - prev_qty).max(0.0);
                            new_liq += delta;
                        }
                        new_liq -= consumed_sell;
                        Self::update_prev_by_price_static(
                            &mut order.prev_by_price,
                            snapshot.bid_levels(),
                            order.price,
                            false,
                        );
                        if new_liq <= 0.0 {
                            0.0
                        } else {
                            let fill = new_liq.min(order.size);
                            consumed_sell += fill;
                            fill
                        }
                    }
                }
            };

            if fill_size > 1e-12 {
                fills.push((cid, fill_size));
            }

            // Process matured cancel AFTER fill opportunity (matching Python)
            if let Some(cancel_at) = order.pending_cancel_at {
                if now >= cancel_at {
                    to_remove.push(cid);
                }
            }
        }

        // Process fills
        for (cid, fill_size) in fills {
            let (side, price, level, remaining) = {
                let order = self.live_orders.get_mut(&cid).unwrap();
                order.size -= fill_size;
                let remaining = order.size;
                (order.side, order.price, order.level, remaining)
            };

            self.process_fill(side, price, fill_size, level, mid_price);

            if remaining < 1e-12 {
                to_remove.push(cid);
            }
        }

        // Remove filled/cancelled orders
        for cid in to_remove {
            self.live_orders.remove(&cid);
        }

        // Update portfolio value: initial + all PnL
        self.portfolio_value = self.initial_portfolio_value + self.realized_pnl + self.unrealized_pnl(mid_price);
    }

    fn update_prev_by_price_static(
        prev: &mut HashMap<u64, f64>,
        levels: &[crate::types::PriceLevel],
        order_price: f64,
        is_buy: bool,
    ) {
        prev.clear();
        for level in levels {
            if is_buy {
                if level.price > order_price { break; }
            } else if level.price < order_price {
                break;
            }
            prev.insert(level.price.to_bits(), level.quantity);
        }
    }

    fn process_fill(&mut self, side: OrderSide, fill_price: f64, fill_size: f64, level: usize, mid_price: f64) {
        let old_position = self.position;
        let fee = fill_size * fill_price * self.maker_fee_rate;

        // Fee deducted from both capital and realized_pnl (matching Python)
        self.available_capital -= fee;
        self.realized_pnl -= fee;

        match side {
            OrderSide::Buy => {
                if old_position < 0.0 {
                    // Reducing short (possibly flipping to long)
                    let reduce_qty = fill_size.min(old_position.abs());
                    if reduce_qty > 0.0 && self.entry_vwap > 0.0 {
                        let pnl = reduce_qty * (self.entry_vwap - fill_price);
                        self.realized_pnl += pnl;
                        let margin_release = reduce_qty * self.entry_vwap / self.leverage as f64;
                        self.available_capital += margin_release + pnl;
                    }
                    let increase_qty = fill_size - reduce_qty;
                    if increase_qty > 0.0 {
                        // Flipped to long: new entry at fill_price
                        self.entry_vwap = fill_price;
                        let margin_needed = increase_qty * fill_price / self.leverage as f64;
                        self.available_capital -= margin_needed;
                    } else if (old_position.abs() - reduce_qty).abs() < 1e-12 {
                        // Fully closed: reset entry
                        self.entry_vwap = 0.0;
                    }
                } else {
                    // Increasing long
                    let new_pos = old_position + fill_size;
                    if new_pos.abs() > 1e-12 {
                        self.entry_vwap = (self.entry_vwap * old_position.abs() + fill_price * fill_size) / new_pos.abs();
                    }
                    let margin_needed = fill_size * fill_price / self.leverage as f64;
                    self.available_capital -= margin_needed;
                }
                self.position += fill_size;
            }
            OrderSide::Sell => {
                if old_position > 0.0 {
                    // Reducing long (possibly flipping to short)
                    let reduce_qty = fill_size.min(old_position);
                    if reduce_qty > 0.0 && self.entry_vwap > 0.0 {
                        let pnl = reduce_qty * (fill_price - self.entry_vwap);
                        self.realized_pnl += pnl;
                        let margin_release = reduce_qty * self.entry_vwap / self.leverage as f64;
                        self.available_capital += margin_release + pnl;
                    }
                    let increase_qty = fill_size - reduce_qty;
                    if increase_qty > 0.0 {
                        // Flipped to short: new entry at fill_price
                        self.entry_vwap = fill_price;
                        let margin_needed = increase_qty * fill_price / self.leverage as f64;
                        self.available_capital -= margin_needed;
                    } else if (old_position - reduce_qty).abs() < 1e-12 {
                        // Fully closed: reset entry
                        self.entry_vwap = 0.0;
                    }
                } else {
                    // Increasing short
                    let new_pos_abs = old_position.abs() + fill_size;
                    if new_pos_abs > 1e-12 {
                        self.entry_vwap = (self.entry_vwap * old_position.abs() + fill_price * fill_size) / new_pos_abs;
                    }
                    let margin_needed = fill_size * fill_price / self.leverage as f64;
                    self.available_capital -= margin_needed;
                }
                self.position -= fill_size;
            }
        }

        self.total_volume += fill_size * fill_price;
        self.fill_count += 1;

        // Log fill
        info!("DRY-RUN FILLED {} L{}: {:.6} @ {:.2} | pos={:.6} | realized=${:.4} | unrealized=${:.4}",
            side, level, fill_size, fill_price, self.position,
            self.realized_pnl, self.unrealized_pnl(mid_price));

        self.trade_logger.log_fill(
            &side.to_string(),
            fill_price,
            fill_size,
            level,
            self.position,
            self.realized_pnl,
            self.available_capital,
            self.portfolio_value,
        );
    }

    pub fn unrealized_pnl(&self, mid_price: f64) -> f64 {
        if self.position.abs() < 1e-12 || self.entry_vwap <= 0.0 {
            return 0.0;
        }
        if self.position > 0.0 {
            self.position * (mid_price - self.entry_vwap)
        } else {
            self.position.abs() * (self.entry_vwap - mid_price)
        }
    }

    pub fn total_pnl(&self, mid_price: f64) -> f64 {
        self.realized_pnl + self.unrealized_pnl(mid_price)
    }

    pub fn live_order_count(&self) -> usize {
        self.live_orders.len()
    }

    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    /// Cancel all live orders immediately (used on disconnect).
    pub fn cancel_all(&mut self) {
        self.live_orders.clear();
    }

    /// Cancel all live orders for a given side.
    pub fn cancel_side(&mut self, side: OrderSide) -> Vec<u64> {
        let now = Instant::now();
        let cids: Vec<u64> = self.live_orders.iter()
            .filter(|(_, o)| o.side == side && o.pending_cancel_at.is_none())
            .map(|(cid, _)| *cid)
            .collect();
        for &cid in &cids {
            if let Some(order) = self.live_orders.get_mut(&cid) {
                order.pending_cancel_at = Some(now + self.sim_latency);
            }
        }
        cids
    }

    /// Get live order IDs and prices for a given side and level.
    pub fn live_orders_for(&self, side: OrderSide, level: usize) -> Vec<(u64, f64)> {
        self.live_orders.iter()
            .filter(|(_, o)| o.side == side && o.level == level && o.pending_cancel_at.is_none())
            .map(|(cid, o)| (*cid, o.price))
            .collect()
    }
}
