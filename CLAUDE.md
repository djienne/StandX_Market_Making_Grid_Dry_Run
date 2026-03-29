# CLAUDE.md

This file provides guidance to Claude Code when working with code in this repository.

## Build and Run Commands

```bash
# Build (debug)
cargo build

# Build (release, optimized)
cargo build --release

# Run with default config
./target/release/standx-dry-run-grid grid_config.json

# Run with fixed duration (seconds)
./target/release/standx-dry-run-grid grid_config.json --duration 3600

# Smoke test (60s warmup, 3min, 6 slots, logs to logs/grid_smoke/)
./target/release/standx-dry-run-grid grid_config.json --smoke-test

# Check results
python3 check_grid_results.py              # logs/grid/
python3 check_grid_results.py logs/grid_smoke/

# Check compilation without building
cargo check

# Format code
cargo fmt

# Lint
cargo clippy
```

## Architecture

```
1 shared WsClient (StandX orderbook WebSocket, no auth)
        ↓ OrderbookSnapshot
  GridRunner.on_book_update()
        ↓ fan-out (sequential, single-threaded)
  ┌─────────┬──────────┬──────────┬─────────┐
  Slot 0    Slot 1     Slot 2     ...  Slot N
  ├─ ObiStrategy (per-slot rolling vol/imbalance with different params)
  ├─ DryRunEngine (simulated orders, delta-fill, PnL)
  └─ TradeLogger (per-slot CSV)
```

Each slot has its own independent strategy, fill engine, position, and PnL.
Grid config specifies parameter axes (e.g. vol_to_half_spread × skew) and the
Cartesian product creates all slots. State is persisted by parameter values,
not slot index, so config changes recover overlapping combos on restart.

### Key Design Patterns

- **No auth/credentials**: Read-only WebSocket for orderbook data only
- **Single-threaded hot path**: All slot processing happens sequentially per book update
- **Delta-fill simulation**: Only fills against genuinely NEW liquidity at each price level
- **POST_ONLY enforcement**: At order creation and again at simulated arrival time
- **Latency simulation**: 50ms default; orders not fillable until eligible_at
- **Average-cost VWAP**: Position tracking with proper flip/close/increase handling

### Module Structure

- `src/main.rs` — Entry point, CLI args (--duration, --smoke-test), tokio event loop
- `src/config.rs` — GridConfig (grid_config.json), WebSocketConfig, StrategyConfig, GridParams
- `src/grid_runner.rs` — GridRunner: slot creation, WS fan-out, warmup, periodic save/summary
- `src/grid_slot.rs` — GridSlot: per-slot state, order management (place/reprice/cancel)
- `src/dry_run_engine.rs` — DryRunEngine: fill simulation, PnL, margin, state persistence
- `src/simulated_order.rs` — SimulatedOrder, BatchOp (Create/Cancel)
- `src/trade_logger.rs` — Buffered per-slot CSV trade logger (zero I/O on hot path)
- `src/summary.rs` — Periodic grid summary table, final results CSV
- `src/strategy/obi.rs` — OBI strategy (adapted from standx, no SharedEquity/SharedSymbolInfo)
- `src/strategy/rolling.rs` — RollingStats, RollingWindow (O(1) incremental mean/std/zscore)
- `src/strategy/quotes.rs` — Quote struct
- `src/websocket/` — WsClient, StandXMessage parsing, reconnection (copied from standx)
- `src/types.rs` — PriceLevel, OrderbookSnapshot, Symbol (copied from standx)

## Configuration

Main config file: `grid_config.json`

Key sections:
- `parameters`: Axes for Cartesian product (vol_to_half_spread, skew, etc.)
- `fixed`: Constants across all slots (min_half_spread_bps, num_levels, etc.)
- `strategy_defaults`: Tick size, lot size, window steps, looking depth
- `websocket`: StandX WebSocket URL

No credentials needed — this is a dry-run simulator with no real order placement.

## Fill Simulation

The DryRunEngine (ported from lighter_MM/dry_run.py) simulates fills using:
1. **Delta-fill**: Only fills against new/increased liquidity per price level
2. **POST_ONLY**: Rejects orders that would immediately match (at creation + arrival)
3. **Latency**: Orders not eligible until created_at + sim_latency (default 50ms)
4. **Cancel latency**: Orders remain fillable during cancel window
5. **Price priority + FIFO**: Most aggressive orders fill first
6. **Avg-cost VWAP**: Position flip splits into reduce + increase portions

## Output

Logs go to `logs/grid/` (normal) or `logs/grid_smoke/` (smoke test):
- `state_{symbol}_{param_key}.json` — Persistent state per slot
- `trades_{symbol}_{param_key}.csv` — Fill history per slot
- `summary.log` — Periodic summary tables
- `results_{symbol}_{timestamp}.csv` — Final results on shutdown
