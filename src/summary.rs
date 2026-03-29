//! Grid summary logging and final results CSV.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::time::Duration;

use chrono::Utc;
use tracing::info;

use crate::grid_slot::GridSlot;

/// Log a periodic grid summary table.
pub fn log_grid_summary(
    slots: &[GridSlot],
    elapsed: Duration,
    mid_price: Option<f64>,
    symbol: &str,
    logs_dir: &str,
) {
    let hours = elapsed.as_secs_f64() / 3600.0;
    let mid_str = mid_price.map(|m| format!("${:.2}", m)).unwrap_or_else(|| "N/A".to_string());

    let mut lines = Vec::new();
    lines.push(format!(
        "\nGRID SUMMARY {} ({} slots, {:.1}h elapsed, mid={})",
        symbol, slots.len(), hours, mid_str
    ));
    lines.push(format!(
        "{:<6} | {:>6} | {:>5} | {:>5} | {:>6} | {:>10} | {:>10} | {:>10} | {:>10}",
        "Slot", "v2hs", "mhbp", "skew", "Fills", "Realized", "Unrealzd", "Total", "Volume"
    ));
    lines.push("-".repeat(100));

    let mid = mid_price.unwrap_or(0.0);

    let mut best_idx = 0;
    let mut best_pnl = f64::NEG_INFINITY;

    for (i, slot) in slots.iter().enumerate() {
        let realized = slot.engine.realized_pnl;
        let unrealized = slot.engine.unrealized_pnl(mid);
        let total = realized + unrealized;
        let volume = slot.engine.total_volume;
        let fills = slot.engine.fill_count;

        if total > best_pnl {
            best_pnl = total;
            best_idx = i;
        }

        lines.push(format!(
            "{:<6} | {:>6.1} | {:>5.1} | {:>5.1} | {:>6} | {:>10.4} | {:>10.4} | {:>10.4} | {:>10.2}",
            slot.label,
            slot.params.vol_to_half_spread,
            slot.params.min_half_spread_bps,
            slot.params.skew,
            fills,
            realized,
            unrealized,
            total,
            volume,
        ));
    }

    if !slots.is_empty() {
        let best = &slots[best_idx];
        lines.push(format!(
            "Best: {} (v2hs={}, mhbp={}, skew={}) total=${:.4}",
            best.label,
            best.params.vol_to_half_spread,
            best.params.min_half_spread_bps,
            best.params.skew,
            best_pnl,
        ));
    }

    let full = lines.join("\n");
    info!("{}", full);

    // Write summary.log (overwrite each time — historical data lives in state files and CSVs)
    let log_dir = logs_dir;
    let _ = fs::create_dir_all(log_dir);
    let log_path = format!("{}/summary.log", log_dir);
    if let Ok(mut file) = OpenOptions::new().create(true).write(true).truncate(true).open(&log_path) {
        let _ = writeln!(file, "[{}]\n{}\n", Utc::now().to_rfc3339(), full);
    }
}

/// Write final results CSV on shutdown.
pub fn write_final_results(
    slots: &[GridSlot],
    elapsed: Duration,
    symbol: &str,
    last_mid: Option<f64>,
    logs_dir: &str,
) {
    let log_dir = logs_dir;
    let _ = fs::create_dir_all(log_dir);
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let path = format!("{}/results_{}_{}.csv", log_dir, symbol, timestamp);

    let mut file = match OpenOptions::new().create(true).write(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("Failed to write results CSV: {}", e);
            return;
        }
    };

    let _ = writeln!(file,
        "slot,param_key,v2hs,mhbp,skew,spread_factor,levels,c1_ticks,\
         fills,realized_pnl,unrealized_pnl,total_pnl,volume,position,\
         available_capital,portfolio_value,elapsed_hours"
    );

    let hours = elapsed.as_secs_f64() / 3600.0;
    let mid = last_mid.unwrap_or(0.0);

    for slot in slots {
        let realized = slot.engine.realized_pnl;
        let unrealized = slot.engine.unrealized_pnl(mid);
        let total = realized + unrealized;

        let _ = writeln!(file,
            "{},{},{},{},{},{},{},{},{},{:.6},{:.6},{:.6},{:.2},{:.8},{:.4},{:.4},{:.2}",
            slot.label,
            slot.param_key,
            slot.params.vol_to_half_spread,
            slot.params.min_half_spread_bps,
            slot.params.skew,
            slot.params.spread_factor_level1,
            slot.params.num_levels,
            slot.params.c1_ticks,
            slot.engine.fill_count,
            realized,
            unrealized,
            total,
            slot.engine.total_volume,
            slot.engine.position,
            slot.engine.available_capital,
            slot.engine.portfolio_value,
            hours,
        );
    }

    info!("Final results written to {}", path);
}
