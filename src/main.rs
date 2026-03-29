//! StandX Dry-Run Grid Simulator
//!
//! Runs N independent paper-trading simulations sharing one WebSocket feed.
//! No credentials or authentication required.
//!
//! # Usage
//!
//! ```bash
//! # Normal run (Ctrl+C to stop):
//! ./standx-dry-run-grid grid_config.json
//!
//! # Run for a fixed duration:
//! ./standx-dry-run-grid grid_config.json --duration 3600
//!
//! # Smoke test (60s warmup, 3 min total, 4 slots, separate logs dir):
//! ./standx-dry-run-grid grid_config.json --smoke-test
//! ```

#![allow(dead_code, unused_imports)]

mod config;
mod types;
mod strategy;
mod websocket;
mod simulated_order;
mod dry_run_engine;
mod trade_logger;
mod grid_slot;
mod grid_runner;
mod summary;

use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

use crate::config::GridConfig;
use crate::grid_runner::GridRunner;
use crate::websocket::{WsClientBuilder, WsEvent, StandXMessage};

#[derive(Parser)]
#[command(name = "standx-dry-run-grid")]
#[command(about = "Grid dry-run simulator for StandX market making strategies")]
struct Args {
    /// Path to grid_config.json
    #[arg(default_value = "grid_config.json")]
    config: String,

    /// Run duration in seconds (0 = run until Ctrl+C)
    #[arg(long, default_value = "0")]
    duration: u64,

    /// Smoke test mode: 60s warmup, 3min total, 4 slots, logs to logs/grid_smoke/
    /// Results are kept separate from normal runs.
    #[arg(long)]
    smoke_test: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize logging
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    fmt::Subscriber::builder()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .init();

    // Load config
    let mut config = GridConfig::from_file(&args.config)?;

    // Apply smoke test overrides
    let duration = if args.smoke_test {
        info!("SMOKE TEST MODE: 60s warmup, 3min duration, 6 slots, logs -> logs/grid_smoke/");
        config.warmup_seconds = 60.0;
        config.summary_interval_seconds = 30.0;
        config.logs_dir = "logs/grid_smoke".to_string();
        // 6 slots: include tight spreads (v2hs=2) to test fill mechanics
        config.parameters.clear();
        config.parameters.insert("vol_to_half_spread".to_string(), vec![2.0, 8.0, 24.0]);
        config.parameters.insert("skew".to_string(), vec![1.0, 3.0]);
        // Override min_half_spread_bps to 1 for tight spread testing
        config.fixed.insert("min_half_spread_bps".to_string(), serde_json::json!(1.0));
        Duration::from_secs(180) // 3 minutes
    } else if args.duration > 0 {
        Duration::from_secs(args.duration)
    } else {
        Duration::ZERO // 0 = run forever
    };

    info!("StandX Dry-Run Grid Simulator starting...");
    info!("Config: symbol={}, capital={}, leverage={}, warmup={}s, latency={}ms",
        config.symbol, config.capital, config.leverage,
        config.warmup_seconds, config.sim_latency_ms);

    let total_slots: usize = config.parameters.values().map(|v| v.len()).product();
    info!("Parameter axes: {} -> {} slots", config.parameters.len(), total_slots);

    if duration > Duration::ZERO {
        info!("Will run for {}s then exit.", duration.as_secs());
    }

    // Create grid runner
    let mut runner = GridRunner::new(config);

    // Create WebSocket client
    let ws_client = WsClientBuilder::new()
        .config(runner.ws_config().clone())
        .symbols(vec![runner.symbol().to_string()])
        .build();

    let mut ws_rx = Arc::clone(&ws_client).run().await;

    // Graceful shutdown on SIGINT/SIGTERM
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    let orderbook_levels = runner.orderbook_levels();
    let mut current_mid: Option<f64>;
    let start = Instant::now();

    info!("Waiting for WebSocket connection...");

    loop {
        // Check duration limit
        if duration > Duration::ZERO && start.elapsed() >= duration {
            info!("Duration limit reached ({}s)", duration.as_secs());
            break;
        }

        tokio::select! {
            Some(event) = ws_rx.recv() => {
                match event {
                    WsEvent::Message(StandXMessage::DepthBook(data), received_at) => {
                        match data.to_snapshot(orderbook_levels, received_at) {
                            Ok(snapshot) => {
                                current_mid = snapshot.mid_price();
                                runner.set_last_mid(current_mid);

                                if !runner.warmed_up() {
                                    runner.feed_warmup(&snapshot);
                                } else {
                                    runner.on_book_update(snapshot);
                                }

                                runner.maybe_periodic(current_mid);
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse snapshot: {}", e);
                            }
                        }
                    }
                    WsEvent::Connected => {
                        info!("WebSocket connected to StandX");
                    }
                    WsEvent::Disconnected(reason) => {
                        tracing::warn!("WebSocket disconnected: {}", reason);
                        runner.on_disconnect();
                    }
                    WsEvent::Error(e) => {
                        tracing::error!("WebSocket error: {}", e);
                    }
                    _ => {}
                }
            }
            _ = &mut shutdown => {
                info!("Shutdown signal received (Ctrl+C)");
                break;
            }
        }
    }

    runner.shutdown();

    if args.smoke_test {
        info!("Smoke test complete. Check logs/grid_smoke/ for results.");
        info!("Run: python3 check_grid_results.py logs/grid_smoke/");
    }

    info!("Goodbye.");
    Ok(())
}
