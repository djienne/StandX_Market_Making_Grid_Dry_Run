//! Grid runner: orchestrates N parallel dry-run slots sharing one WS feed.

use std::time::Instant;

use tracing::info;

use crate::config::GridConfig;
use crate::grid_slot::GridSlot;
use crate::summary;
use crate::types::OrderbookSnapshot;

pub struct GridRunner {
    pub config: GridConfig,
    pub slots: Vec<GridSlot>,
    start_time: Instant,
    last_summary: Instant,
    last_save: Instant,
    book_update_count: u64,
    warmed_up: bool,
    warmup_logged: bool,
    warmup_start: Instant,
    last_warmup_progress: Instant,
    last_mid: Option<f64>,
}

impl GridRunner {
    pub fn new(config: GridConfig) -> Self {
        let params_list = config.build_params();
        let num_slots = params_list.len();

        info!("Creating {} grid slots for {}", num_slots, config.symbol);

        let slots: Vec<GridSlot> = params_list
            .into_iter()
            .enumerate()
            .map(|(i, params)| {
                let slot = GridSlot::new(i, params, &config);
                info!("  {} | {} | v2hs={} skew={} mhbp={} levels={} c1t={}",
                    slot.label, slot.param_key,
                    slot.params.vol_to_half_spread,
                    slot.params.skew,
                    slot.params.min_half_spread_bps,
                    slot.params.num_levels,
                    slot.params.c1_ticks);
                slot
            })
            .collect();

        let now = Instant::now();
        Self {
            config,
            slots,
            start_time: now,
            last_summary: now,
            last_save: now,
            book_update_count: 0,
            warmed_up: false,
            warmup_logged: false,
            warmup_start: now,
            last_warmup_progress: now,
            last_mid: None,
        }
    }

    pub fn symbol(&self) -> &str { &self.config.symbol }
    pub fn ws_config(&self) -> &crate::config::WebSocketConfig { &self.config.websocket }
    pub fn orderbook_levels(&self) -> usize { self.config.orderbook_levels }

    pub fn warmed_up(&self) -> bool {
        self.warmed_up
    }

    /// Feed orderbook to all slots during warmup (strategies accumulate data, no orders).
    pub fn feed_warmup(&mut self, snapshot: &OrderbookSnapshot) {
        if !self.warmup_logged {
            self.warmup_logged = true;
            info!("Warmup started ({:.0}s). Accumulating data, no orders placed yet.",
                self.config.warmup_seconds);
        }
        for slot in &mut self.slots {
            slot.strategy.set_position(slot.engine.position);
            let _ = slot.strategy.update(snapshot);
        }

        let elapsed = self.warmup_start.elapsed().as_secs_f64();
        if elapsed >= self.config.warmup_seconds {
            self.warmed_up = true;
            info!("Warmup complete ({:.0}s). Starting grid dry-run with {} slots.",
                elapsed, self.slots.len());
        } else if self.last_warmup_progress.elapsed().as_secs() >= 60 {
            self.last_warmup_progress = Instant::now();
            let pct = (elapsed / self.config.warmup_seconds * 100.0) as u32;
            let remaining = (self.config.warmup_seconds - elapsed) as u64;
            info!("Warmup {}% — {:.0}s / {:.0}s ({remaining}s remaining)",
                pct, elapsed, self.config.warmup_seconds);
        }
    }

    /// Fan-out: process a new orderbook snapshot across all slots.
    pub fn on_book_update(&mut self, snapshot: OrderbookSnapshot) {
        self.book_update_count += 1;

        for slot in &mut self.slots {
            slot.on_book_update(&snapshot);
        }
    }

    /// Periodic operations: summary logging, state persistence.
    pub fn maybe_periodic(&mut self, mid_price: Option<f64>) {
        let now = Instant::now();

        // Summary logging
        let summary_interval = std::time::Duration::from_secs_f64(self.config.summary_interval_seconds);
        if now.duration_since(self.last_summary) >= summary_interval {
            self.last_summary = now;
            let elapsed = self.start_time.elapsed();
            summary::log_grid_summary(&self.slots, elapsed, mid_price, &self.config.symbol, &self.config.logs_dir, self.warmed_up);
        }

        // State persistence (every 60s)
        let save_interval = std::time::Duration::from_secs(60);
        if now.duration_since(self.last_save) >= save_interval {
            self.last_save = now;
            for slot in &mut self.slots {
                slot.engine.save_state();
                slot.engine.trade_logger.flush();
            }
        }
    }

    /// On WebSocket disconnect: cancel live orders and reset strategies.
    pub fn on_disconnect(&mut self) {
        info!("WebSocket disconnected, cancelling orders and resetting strategies");
        self.warmed_up = false;
        self.warmup_logged = false;
        self.warmup_start = Instant::now();
        self.last_warmup_progress = Instant::now();
        for slot in &mut self.slots {
            slot.engine.cancel_all();
            slot.strategy.reset_state();
        }
    }

    /// Track last known mid price for final reporting.
    pub fn set_last_mid(&mut self, mid: Option<f64>) {
        self.last_mid = mid;
    }

    /// Graceful shutdown: save all states, flush logs, write final results.
    pub fn shutdown(&mut self) {
        info!("Shutting down grid runner...");

        let elapsed = self.start_time.elapsed();

        for slot in &mut self.slots {
            slot.engine.save_state();
            slot.engine.trade_logger.flush();
        }

        summary::write_final_results(&self.slots, elapsed, &self.config.symbol, self.last_mid, &self.config.logs_dir);
        info!("Grid runner shut down. {} book updates processed.", self.book_update_count);
    }
}
