# StandX Dry-Run Grid Simulator

Runs **N independent paper-trading simulations** sharing **one WebSocket feed** to StandX.
No credentials or authentication required. Ported from the lighter_MM Python grid dry-run.

## Quick Start

```bash
# Build
cargo build --release

# Run (136 slots, Ctrl+C to stop):
./target/release/standx-dry-run-grid grid_config.json

# Run for a fixed duration (e.g. 3 hours):
./target/release/standx-dry-run-grid grid_config.json --duration 10800

# Smoke test (60s warmup, 3min total, 6 slots, separate logs dir):
./target/release/standx-dry-run-grid grid_config.json --smoke-test
```

## Smoke Test

The `--smoke-test` flag is a quick validation mode:
- **60s warmup** (instead of 600s)
- **3 minute total runtime** (auto-exits)
- **6 slots** (3 vol_to_half_spread × 2 skew, includes tight v2hs=2 for fill testing)
- **Logs to `logs/grid_smoke/`** — does NOT pollute normal `logs/grid/` data
- **Summary every 30s** (instead of 60s)

Use it to verify the system connects, receives data, and places/fills simulated orders
before committing to a long run.

```bash
# Run smoke test
./target/release/standx-dry-run-grid grid_config.json --smoke-test

# Check smoke test results
python3 check_grid_results.py logs/grid_smoke/
```

Note: fills may be sparse during smoke tests on quiet markets (BTC needs price
movement to trigger delta-fills). Zero fills in 3 minutes is normal on low-activity
periods — it means orders were placed but the book didn't refresh at those levels.

## Check Results

```bash
# Default (scan logs/grid/)
python3 check_grid_results.py

# Custom directory
python3 check_grid_results.py logs/grid_smoke/

# Top 20 performers
python3 check_grid_results.py --top 20

# Sort by fills, volume, or efficiency
python3 check_grid_results.py --sort fills
python3 check_grid_results.py --sort efficiency
```

Output includes:
- Overall summary (slots, fills, volume, profitable/losing)
- Top N / Bottom N table
- Parameter analysis (avg PnL per vol_to_half_spread and skew value)
- PnL heatmap (vol_to_half_spread × skew)

## Configuration

Edit `grid_config.json`:

```json
{
  "symbol": "BTC-USD",
  "capital": 1000,
  "leverage": 1,
  "warmup_seconds": 600,
  "summary_interval_seconds": 60,
  "sim_latency_ms": 50,
  "maker_fee_rate": 0.0001,
  "parameters": {
    "vol_to_half_spread": [4, 6, 8, 10, 12, 15, 18, 21, 24, 30, 36, 42, 48, 54, 60, 70, 80],
    "skew": [0.1, 0.5, 1.0, 1.5, 2.5, 3.0, 4.0, 5.0]
  },
  "fixed": {
    "min_half_spread_bps": 2.0,
    "spread_factor_level1": 2.0,
    "num_levels": 2,
    "c1_ticks": 20.0
  }
}
```

**parameters**: Axes for Cartesian product (17 × 8 = 136 slots above).
**fixed**: Constant across all slots.

## Output

All output goes to the configured `logs_dir` (default `logs/grid/`):

| File | Description |
|------|-------------|
| `state_{symbol}_{param_key}.json` | Per-slot persistent state (position, PnL, capital) |
| `trades_{symbol}_{param_key}.csv` | Per-slot fill history |
| `summary.log` | Periodic grid summary tables |
| `results_{symbol}_{timestamp}.csv` | Final results CSV (written on shutdown) |

State files are keyed by **parameter values** — changing the grid config recovers
overlapping parameter combos automatically on restart.

## Resource Usage

With 136 slots on BTC-USD:
- **RAM**: ~18 MB RSS
- **CPU**: <1% (single-threaded hot path)
- **Disk**: ~700 KB/hour (state JSONs + summary log, trade CSVs grow with fills)
- **Network**: 1 WebSocket connection (~10 KB/s)

## Architecture

```
1 shared WsClient (StandX orderbook WebSocket)
        ↓ OrderbookSnapshot
  GridRunner.on_book_update()
        ↓ fan-out (sequential, single-threaded)
  ┌─────────┬──────────┬──────────┬─────────┐
  Slot 0    Slot 1     Slot 2     ...  Slot N
  ├─ ObiStrategy (per-slot rolling vol/imbalance)
  ├─ DryRunEngine (simulated orders, fills, PnL)
  └─ TradeLogger (per-slot CSV)
```

## StandX Fees

- Maker fee: 0.01% (default in config)
- Taker fee: 0.04% (not used — all simulated orders are maker/POST_ONLY)
