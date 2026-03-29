# Quick Start Guide

## Prerequisites

- **Rust** (1.70+): https://rustup.rs/
- **Python 3** (for checking results)
- Internet connection (WebSocket to StandX public orderbook)

## 1. Build

```bash
git clone https://github.com/djienne/StandX_Market_Making_Grid_Dry_Run.git
cd StandX_Market_Making_Grid_Dry_Run
cargo build --release
```

## 2. Smoke Test (recommended first run)

```bash
cargo run --release -- --smoke-test
```

This runs a quick 3-minute test with 6 slots and 60s warmup. Logs go to `logs/grid_smoke/`.
Check results:

```bash
python3 check_grid_results.py logs/grid_smoke/
```

Zero fills in 3 minutes is normal on quiet markets — it means the book wasn't active enough at quote levels.

## 3. Full Grid Run

```bash
cargo run --release
```

- Reads `grid_config.json` by default (or pass a custom path: `cargo run --release -- my_config.json`)
- Runs 136 slots (17 vol_to_half_spread x 8 skew combinations)
- 10-minute warmup to build rolling volatility estimates
- Fills start appearing after warmup
- Press **Ctrl+C** to stop gracefully — state is saved automatically

### Run for a fixed duration

```bash
# Run for 3 hours
cargo run --release -- --duration 10800
```

### Run in background

```bash
nohup cargo run --release > grid.log 2>&1 &
echo $!  # save the PID
```

## 4. Check Results

```bash
# Summary with top/bottom 10, parameter analysis, and heatmap
python3 check_grid_results.py

# Show top 20
python3 check_grid_results.py --top 20

# Sort by efficiency (PnL per unit volume in bps)
python3 check_grid_results.py --sort efficiency
```

## 5. Stop and Resume

State is persisted per parameter combination in `logs/grid/state_*.json`. On restart, all slots recover their position, PnL, fill count, and capital. You can stop and restart anytime without losing progress.

To start fresh, delete the state files:

```bash
rm logs/grid/state_*.json logs/grid/trades_*.csv logs/grid/summary.log
```

## 6. Customize Parameters

Edit `grid_config.json`:

| Field | Description | Default |
|-------|-------------|---------|
| `vol_to_half_spread` | Multiples of rolling vol for half-spread (array) | `[4, 6, 8, ...]` |
| `skew` | Inventory skew aggressiveness (array) | `[0.1, 0.5, 1.0, ...]` |
| `min_half_spread_bps` | Floor for half-spread in basis points | `2.0` |
| `capital` | Simulated capital per slot ($) | `1000` |
| `warmup_seconds` | Seconds to collect vol data before trading | `600` |
| `sim_latency_ms` | Simulated order latency (ms) | `50` |

The grid runs the Cartesian product of `vol_to_half_spread` x `skew`, so 17 x 8 = 136 slots.

## Resource Usage

- **RAM**: ~18 MB
- **CPU**: <1%
- **Disk**: ~1 MB/hour
- **Network**: 1 WebSocket connection (~10 KB/s)

No API keys, credentials, or authentication needed.
