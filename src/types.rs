//! Core data types for orderbook snapshots.
//!
//! Copied from standx with no modifications.
//! Fixed-size arrays for price levels, no heap allocations in hot paths.

use std::fmt;

/// Maximum number of price levels supported.
pub const MAX_LEVELS: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PriceOrder {
    Ascending,
    Descending,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[repr(C)]
pub struct PriceLevel {
    pub price: f64,
    pub quantity: f64,
}

impl PriceLevel {
    #[inline]
    pub const fn new(price: f64, quantity: f64) -> Self {
        Self { price, quantity }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.price == 0.0 && self.quantity == 0.0
    }

    #[inline]
    pub fn notional(&self) -> f64 {
        self.price * self.quantity
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol {
    data: [u8; 16],
    len: u8,
}

impl Symbol {
    pub fn new(s: &str) -> Self {
        let bytes = s.as_bytes();
        let len = bytes.len().min(15) as u8;
        let mut data = [0u8; 16];
        data[..len as usize].copy_from_slice(&bytes[..len as usize]);
        Self { data, len }
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.data[..self.len as usize]) }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for Symbol {
    fn default() -> Self {
        Self { data: [0u8; 16], len: 0 }
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Symbol(\"{}\")", self.as_str())
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self { Self::new(s) }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str { self.as_str() }
}

#[derive(Clone)]
pub struct OrderbookSnapshot {
    pub symbol: Symbol,
    pub timestamp_ns: i64,
    pub received_at_ns: i64,
    pub sequence: u64,
    pub bids: [PriceLevel; MAX_LEVELS],
    pub asks: [PriceLevel; MAX_LEVELS],
    pub bid_count: u8,
    pub ask_count: u8,
}

impl Default for OrderbookSnapshot {
    fn default() -> Self {
        Self {
            symbol: Symbol::default(),
            timestamp_ns: 0,
            received_at_ns: 0,
            sequence: 0,
            bids: [PriceLevel::default(); MAX_LEVELS],
            asks: [PriceLevel::default(); MAX_LEVELS],
            bid_count: 0,
            ask_count: 0,
        }
    }
}

impl OrderbookSnapshot {
    pub fn new(symbol: Symbol) -> Self {
        Self { symbol, ..Default::default() }
    }

    #[inline]
    pub fn best_bid(&self) -> Option<PriceLevel> {
        if self.bid_count > 0 { Some(self.bids[0]) } else { None }
    }

    #[inline]
    pub fn best_ask(&self) -> Option<PriceLevel> {
        if self.ask_count > 0 { Some(self.asks[0]) } else { None }
    }

    #[inline]
    pub fn best_bid_price(&self) -> Option<f64> {
        self.best_bid().map(|l| l.price)
    }

    #[inline]
    pub fn best_ask_price(&self) -> Option<f64> {
        self.best_ask().map(|l| l.price)
    }

    #[inline]
    pub fn spread(&self) -> Option<f64> {
        match (self.best_bid_price(), self.best_ask_price()) {
            (Some(bid), Some(ask)) => Some(ask - bid),
            _ => None,
        }
    }

    #[inline]
    pub fn mid_price(&self) -> Option<f64> {
        match (self.best_bid_price(), self.best_ask_price()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
            _ => None,
        }
    }

    #[inline]
    pub fn spread_bps(&self) -> Option<f64> {
        match (self.spread(), self.mid_price()) {
            (Some(spread), Some(mid)) if mid > 0.0 => Some(spread / mid * 10000.0),
            _ => None,
        }
    }

    #[inline]
    pub fn bid_levels(&self) -> &[PriceLevel] {
        &self.bids[..self.bid_count as usize]
    }

    #[inline]
    pub fn ask_levels(&self) -> &[PriceLevel] {
        &self.asks[..self.ask_count as usize]
    }

    pub fn total_bid_volume(&self) -> f64 {
        self.bid_levels().iter().map(|l| l.quantity).sum()
    }

    pub fn total_ask_volume(&self) -> f64 {
        self.ask_levels().iter().map(|l| l.quantity).sum()
    }

    pub fn is_valid(&self) -> bool {
        if self.bid_count == 0 || self.ask_count == 0 {
            return false;
        }
        if let (Some(bid), Some(ask)) = (self.best_bid_price(), self.best_ask_price()) {
            if bid >= ask { return false; }
        }
        true
    }

    #[inline]
    fn detect_price_order(levels: &[(String, String)]) -> PriceOrder {
        let mut prev: Option<f64> = None;
        let mut direction = PriceOrder::Unknown;
        for (price_str, _) in levels.iter() {
            let price = match fast_float::parse::<f64, _>(price_str) {
                Ok(price) if price > 0.0 => price,
                _ => continue,
            };
            if let Some(prev_price) = prev {
                if direction == PriceOrder::Unknown {
                    if price > prev_price { direction = PriceOrder::Ascending; }
                    else if price < prev_price { direction = PriceOrder::Descending; }
                } else if (direction == PriceOrder::Ascending && price < prev_price)
                    || (direction == PriceOrder::Descending && price > prev_price) {
                    return PriceOrder::Unknown;
                }
            }
            prev = Some(price);
        }
        direction
    }

    #[inline]
    pub fn set_bids_from_strings(&mut self, levels: &[(String, String)], max_levels: usize) {
        let order = Self::detect_price_order(levels);
        let mut count = 0;
        let limit = max_levels.min(MAX_LEVELS);
        match order {
            PriceOrder::Ascending => {
                for (price_str, qty_str) in levels.iter().rev() {
                    if count >= limit { break; }
                    if let (Ok(price), Ok(qty)) = (
                        fast_float::parse::<f64, _>(price_str),
                        fast_float::parse::<f64, _>(qty_str),
                    ) {
                        if price > 0.0 && qty > 0.0 {
                            self.bids[count] = PriceLevel::new(price, qty);
                            count += 1;
                        }
                    }
                }
            }
            PriceOrder::Descending => {
                for (price_str, qty_str) in levels.iter() {
                    if count >= limit { break; }
                    if let (Ok(price), Ok(qty)) = (
                        fast_float::parse::<f64, _>(price_str),
                        fast_float::parse::<f64, _>(qty_str),
                    ) {
                        if price > 0.0 && qty > 0.0 {
                            self.bids[count] = PriceLevel::new(price, qty);
                            count += 1;
                        }
                    }
                }
            }
            PriceOrder::Unknown => {
                for (price_str, qty_str) in levels.iter() {
                    if let (Ok(price), Ok(qty)) = (
                        fast_float::parse::<f64, _>(price_str),
                        fast_float::parse::<f64, _>(qty_str),
                    ) {
                        if price > 0.0 && qty > 0.0 {
                            if count < limit {
                                self.bids[count] = PriceLevel::new(price, qty);
                                count += 1;
                            } else if limit > 0 {
                                let mut worst_idx = 0;
                                let mut worst_price = self.bids[0].price;
                                for i in 1..count {
                                    if self.bids[i].price < worst_price {
                                        worst_price = self.bids[i].price;
                                        worst_idx = i;
                                    }
                                }
                                if price > worst_price {
                                    self.bids[worst_idx] = PriceLevel::new(price, qty);
                                }
                            }
                        }
                    }
                }
            }
        }
        self.bid_count = count as u8;
        self.bids[..count].sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
        for i in count..MAX_LEVELS { self.bids[i] = PriceLevel::default(); }
    }

    #[inline]
    pub fn set_asks_from_strings(&mut self, levels: &[(String, String)], max_levels: usize) {
        let order = Self::detect_price_order(levels);
        let mut count = 0;
        let limit = max_levels.min(MAX_LEVELS);
        match order {
            PriceOrder::Ascending => {
                for (price_str, qty_str) in levels.iter() {
                    if count >= limit { break; }
                    if let (Ok(price), Ok(qty)) = (
                        fast_float::parse::<f64, _>(price_str),
                        fast_float::parse::<f64, _>(qty_str),
                    ) {
                        if price > 0.0 && qty > 0.0 {
                            self.asks[count] = PriceLevel::new(price, qty);
                            count += 1;
                        }
                    }
                }
            }
            PriceOrder::Descending => {
                for (price_str, qty_str) in levels.iter().rev() {
                    if count >= limit { break; }
                    if let (Ok(price), Ok(qty)) = (
                        fast_float::parse::<f64, _>(price_str),
                        fast_float::parse::<f64, _>(qty_str),
                    ) {
                        if price > 0.0 && qty > 0.0 {
                            self.asks[count] = PriceLevel::new(price, qty);
                            count += 1;
                        }
                    }
                }
            }
            PriceOrder::Unknown => {
                for (price_str, qty_str) in levels.iter() {
                    if let (Ok(price), Ok(qty)) = (
                        fast_float::parse::<f64, _>(price_str),
                        fast_float::parse::<f64, _>(qty_str),
                    ) {
                        if price > 0.0 && qty > 0.0 {
                            if count < limit {
                                self.asks[count] = PriceLevel::new(price, qty);
                                count += 1;
                            } else if limit > 0 {
                                let mut worst_idx = 0;
                                let mut worst_price = self.asks[0].price;
                                for i in 1..count {
                                    if self.asks[i].price > worst_price {
                                        worst_price = self.asks[i].price;
                                        worst_idx = i;
                                    }
                                }
                                if price < worst_price {
                                    self.asks[worst_idx] = PriceLevel::new(price, qty);
                                }
                            }
                        }
                    }
                }
            }
        }
        self.ask_count = count as u8;
        self.asks[..count].sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
        for i in count..MAX_LEVELS { self.asks[i] = PriceLevel::default(); }
    }
}

impl fmt::Debug for OrderbookSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrderbookSnapshot")
            .field("symbol", &self.symbol)
            .field("sequence", &self.sequence)
            .field("bid_count", &self.bid_count)
            .field("ask_count", &self.ask_count)
            .field("best_bid", &self.best_bid_price())
            .field("best_ask", &self.best_ask_price())
            .field("spread", &self.spread())
            .finish()
    }
}
