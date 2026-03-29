//! Simulated order types for the dry-run engine.

use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl std::fmt::Display for OrderSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderSide::Buy => write!(f, "buy"),
            OrderSide::Sell => write!(f, "sell"),
        }
    }
}

/// A simulated order that lives only in memory (never sent to the exchange).
pub struct SimulatedOrder {
    pub client_order_id: u64,
    pub side: OrderSide,
    pub price: f64,
    pub size: f64,
    pub original_size: f64,
    pub level: usize,
    pub created_at: Instant,
    /// Order is not fillable until this time (simulated latency).
    pub eligible_at: Instant,
    /// If set, the order will be removed after this time (cancel latency).
    pub pending_cancel_at: Option<Instant>,
    /// Per-price liquidity snapshot at creation for delta-fill.
    /// Key = f64::to_bits(), Value = quantity seen at that price.
    pub prev_by_price: HashMap<u64, f64>,
    /// Whether the POST_ONLY arrival recheck has been done.
    pub arrival_checked: bool,
}

/// Batch operation: create, modify, or cancel a simulated order.
#[derive(Debug)]
pub enum BatchOp {
    Create {
        side: OrderSide,
        price: f64,
        size: f64,
        level: usize,
    },
    Cancel {
        client_order_id: u64,
    },
}
