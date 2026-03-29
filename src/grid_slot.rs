//! Per-slot state container for the grid dry-run.

use std::path::PathBuf;

use crate::config::{GridConfig, GridParams};
use crate::dry_run_engine::DryRunEngine;
use crate::simulated_order::{BatchOp, OrderSide};
use crate::strategy::ObiStrategy;
use crate::strategy::Quote;
use crate::trade_logger::TradeLogger;
use crate::types::OrderbookSnapshot;

pub struct GridSlot {
    pub index: usize,
    pub label: String,
    pub param_key: String,
    pub params: GridParams,
    pub strategy: ObiStrategy,
    pub engine: DryRunEngine,
}

impl GridSlot {
    pub fn new(
        index: usize,
        params: GridParams,
        config: &GridConfig,
    ) -> Self {
        let label = format!("s{:03}", index);
        let param_key = params.param_key();

        let strategy_config = config.build_strategy_config(&params);
        let strategy = ObiStrategy::new(strategy_config, config.warmup_seconds);

        // Compute order sizing
        let capital = config.capital;
        let leverage = config.leverage;
        let order_levels = params.num_levels;
        // order_qty_dollar = (capital * leverage / 5 * 0.9) / order_levels
        let order_qty_dollar = (capital * leverage as f64 / 5.0 * 0.9) / order_levels as f64;
        let max_position_dollar = capital * leverage as f64 * 0.9;

        let logs_dir = PathBuf::from(&config.logs_dir);
        let state_path = logs_dir.join(format!("state_{}_{}.json", config.symbol, param_key));
        let trade_path = logs_dir.join(format!("trades_{}_{}.csv", config.symbol, param_key));

        let trade_logger = TradeLogger::new(trade_path, &config.symbol);

        let slot_cid_base = (index as u64 + 1) * 1_000_000;

        let mut engine = DryRunEngine::new(
            capital,
            leverage,
            config.sim_latency_ms,
            config.maker_fee_rate,
            state_path,
            trade_logger,
            slot_cid_base,
        );

        // Try to restore persisted state
        engine.try_load_state();

        let mut slot = Self {
            index,
            label,
            param_key,
            params,
            strategy,
            engine,
        };

        // Set sizing on strategy
        slot.strategy.set_order_qty_dollar(order_qty_dollar);
        slot.strategy.set_max_position_dollar(max_position_dollar);

        slot
    }

    /// Feed orderbook to strategy (warmup or quoting), check fills, maybe place orders.
    pub fn on_book_update(&mut self, snapshot: &OrderbookSnapshot) {
        // Sync position from engine to strategy
        self.strategy.set_position(self.engine.position);

        // Feed to strategy
        let quote = self.strategy.update(snapshot);

        // Check fills against current book
        self.engine.check_fills(snapshot);

        // If strategy produced a valid-for-trading quote, manage orders
        if let Some(ref q) = quote {
            if q.valid_for_trading {
                self.manage_orders(q, snapshot);
            }
        }

    }

    /// Place/replace simulated orders based on the current quote.
    fn manage_orders(&mut self, quote: &Quote, snapshot: &OrderbookSnapshot) {
        let mut ops: Vec<BatchOp> = Vec::new();
        let reprice_threshold_bps = 1.0; // 1 bps reprice threshold

        for level in 0..quote.num_levels {
            let bid_price = quote.bid_prices[level];
            let ask_price = quote.ask_prices[level];

            if bid_price <= 0.0 || ask_price <= 0.0 || bid_price >= ask_price {
                continue;
            }

            // Check existing orders for this level
            let bid_orders = self.engine.live_orders_for(OrderSide::Buy, level);
            let ask_orders = self.engine.live_orders_for(OrderSide::Sell, level);

            // Bid management
            if bid_orders.is_empty() {
                ops.push(BatchOp::Create {
                    side: OrderSide::Buy,
                    price: bid_price,
                    size: quote.quantity,
                    level,
                });
            } else {
                for (cid, existing_price) in &bid_orders {
                    let diff_bps = ((bid_price - existing_price) / existing_price).abs() * 10000.0;
                    if diff_bps > reprice_threshold_bps {
                        ops.push(BatchOp::Cancel { client_order_id: *cid });
                        ops.push(BatchOp::Create {
                            side: OrderSide::Buy,
                            price: bid_price,
                            size: quote.quantity,
                            level,
                        });
                    }
                }
            }

            // Ask management
            if ask_orders.is_empty() {
                ops.push(BatchOp::Create {
                    side: OrderSide::Sell,
                    price: ask_price,
                    size: quote.quantity,
                    level,
                });
            } else {
                for (cid, existing_price) in &ask_orders {
                    let diff_bps = ((ask_price - existing_price) / existing_price).abs() * 10000.0;
                    if diff_bps > reprice_threshold_bps {
                        ops.push(BatchOp::Cancel { client_order_id: *cid });
                        ops.push(BatchOp::Create {
                            side: OrderSide::Sell,
                            price: ask_price,
                            size: quote.quantity,
                            level,
                        });
                    }
                }
            }
        }

        if !ops.is_empty() {
            self.engine.process_batch(&ops, snapshot);
        }
    }
}
