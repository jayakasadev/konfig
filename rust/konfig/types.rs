//! Owned Rust snapshot types for trading configuration.
//!
//! All fields are pre-decoded primitives — no FlatBuffers borrows.
//! This satisfies `Send + 'static` and is safe to store in [`ArcSwap`].
//!
//! [`ArcSwap`]: arc_swap::ArcSwap

use std::time::Instant;

use thiserror::Error;

// ── FlatBuffers vtable slot offsets ──────────────────────────────────────────
//
// Matches `schema/konfig/trading_config.fbs` (namespace konfig.v1):
//
//   table RiskParams {
//     slot 4:  max_position_usd:double
//     slot 6:  max_order_size_usd:double
//     slot 8:  max_daily_loss_usd:double
//     slot 10: max_orders_per_second:uint32
//     slot 12: max_notional_per_minute:double
//     slot 14: enabled:bool = true
//   }
//
//   table StrategyParams {
//     slot 4:  product_id:string
//     slot 6:  signal_threshold:double
//     slot 8:  lookback_window_ms:uint64
//     slot 10: max_spread_bps:double
//     slot 12: enabled:bool = true
//   }
//
//   table TradingConfig {
//     slot 4:  schema_version:uint32
//     slot 6:  risk:RiskParams
//     slot 8:  strategies:[StrategyParams]
//   }

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("FlatBuffers payload is empty")]
    EmptyPayload,
    #[error("FlatBuffers table field missing or out of range: slot {0}")]
    MissingField(u16),
}

// ── Snapshot types ────────────────────────────────────────────────────────────

/// Pre-decoded risk parameters — no allocation on read.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskParamsSnapshot {
    pub max_position_usd: f64,
    pub max_order_size_usd: f64,
    pub max_daily_loss_usd: f64,
    pub max_orders_per_second: u32,
    pub max_notional_per_minute: f64,
    pub enabled: bool,
}

impl Default for RiskParamsSnapshot {
    fn default() -> Self {
        Self {
            max_position_usd: 0.0,
            max_order_size_usd: 0.0,
            max_daily_loss_usd: 0.0,
            max_orders_per_second: 0,
            max_notional_per_minute: 0.0,
            enabled: true,
        }
    }
}

/// Pre-decoded per-strategy parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyParamsSnapshot {
    pub product_id: String,
    pub signal_threshold: f64,
    pub lookback_window_ms: u64,
    pub max_spread_bps: f64,
    pub enabled: bool,
}

impl Default for StrategyParamsSnapshot {
    fn default() -> Self {
        Self {
            product_id: String::new(),
            signal_threshold: 0.0,
            lookback_window_ms: 0,
            max_spread_bps: 0.0,
            enabled: true,
        }
    }
}

/// Complete trading configuration snapshot — cheap to clone, cheap to read.
///
/// `resource_version` and `loaded_at` are watcher metadata; they are not
/// part of the FlatBuffers schema.
#[derive(Debug, Clone)]
pub struct TradingConfigSnapshot {
    pub schema_version: u32,
    pub risk: RiskParamsSnapshot,
    pub strategies: Vec<StrategyParamsSnapshot>,
    pub resource_version: String,
    pub loaded_at: Instant,
}

impl Default for TradingConfigSnapshot {
    fn default() -> Self {
        Self {
            schema_version: 0,
            risk: RiskParamsSnapshot::default(),
            strategies: Vec::new(),
            resource_version: String::new(),
            loaded_at: Instant::now(),
        }
    }
}

// ── FlatBuffers helpers ───────────────────────────────────────────────────────

impl TradingConfigSnapshot {
    /// Decode a `TradingConfig` FlatBuffers buffer into an owned snapshot.
    ///
    /// Reads `schema_version`, the nested `risk` table, and the `strategies`
    /// vector.  All fields are eagerly decoded into owned Rust values so the
    /// resulting struct is `Send + 'static`.
    ///
    /// # Safety
    /// `bytes` must be a valid FlatBuffers buffer whose root type is `TradingConfig`.
    pub unsafe fn from_flatbuffers(
        bytes: &[u8],
        resource_version: String,
    ) -> Result<Self, ParseError> {
        if bytes.is_empty() {
            return Err(ParseError::EmptyPayload);
        }

        // SAFETY: caller guarantees bytes is a valid TradingConfig FlatBuffers buffer.
        let root = unsafe { flatbuffers::root_unchecked::<flatbuffers::Table>(bytes) };

        let schema_version = unsafe { root.get::<u32>(4, Some(0)).unwrap_or(0) };

        // Decode nested RiskParams table (slot 6).
        let risk = unsafe {
            match root.get::<flatbuffers::ForwardsUOffset<flatbuffers::Table>>(6, None) {
                Some(risk_table) => RiskParamsSnapshot {
                    max_position_usd: risk_table.get::<f64>(4, Some(0.0)).unwrap_or(0.0),
                    max_order_size_usd: risk_table.get::<f64>(6, Some(0.0)).unwrap_or(0.0),
                    max_daily_loss_usd: risk_table.get::<f64>(8, Some(0.0)).unwrap_or(0.0),
                    max_orders_per_second: risk_table.get::<u32>(10, Some(0)).unwrap_or(0),
                    max_notional_per_minute: risk_table.get::<f64>(12, Some(0.0)).unwrap_or(0.0),
                    // FlatBuffers default for enabled = true (slot 14).
                    enabled: risk_table.get::<bool>(14, Some(true)).unwrap_or(true),
                },
                None => RiskParamsSnapshot::default(),
            }
        };

        // Decode strategies vector (slot 8 — vector of StrategyParams tables).
        // A missing vector field is treated as an empty slice (valid per FlatBuffers spec).
        let strategies = unsafe {
            match root.get::<flatbuffers::ForwardsUOffset<
                flatbuffers::Vector<flatbuffers::ForwardsUOffset<flatbuffers::Table>>,
            >>(8, None)
            {
                Some(vec) => {
                    let mut out = Vec::with_capacity(vec.len());
                    for i in 0..vec.len() {
                        let t = vec.get(i);
                        let product_id = t
                            .get::<flatbuffers::ForwardsUOffset<&str>>(4, None)
                            .map(|s| s.to_owned())
                            .unwrap_or_default();
                        let signal_threshold = t.get::<f64>(6, Some(0.0)).unwrap_or(0.0);
                        let lookback_window_ms = t.get::<u64>(8, Some(0)).unwrap_or(0);
                        let max_spread_bps = t.get::<f64>(10, Some(0.0)).unwrap_or(0.0);
                        let enabled = t.get::<bool>(12, Some(true)).unwrap_or(true);
                        out.push(StrategyParamsSnapshot {
                            product_id,
                            signal_threshold,
                            lookback_window_ms,
                            max_spread_bps,
                            enabled,
                        });
                    }
                    out
                }
                None => Vec::new(),
            }
        };

        Ok(Self {
            schema_version,
            risk,
            strategies,
            resource_version,
            loaded_at: Instant::now(),
        })
    }
}

// ── FlatBuffers builders (for tests) ─────────────────────────────────────────

/// Build a `TradingConfig` FlatBuffers buffer with a single strategy entry.
///
/// Used in unit tests and internal round-trip verification only.
#[cfg(test)]
pub(crate) fn build_trading_config(
    schema_version: u32,
    max_position_usd: f64,
    max_order_size_usd: f64,
    max_daily_loss_usd: f64,
    max_orders_per_second: u32,
    max_notional_per_minute: f64,
    risk_enabled: bool,
    product_id: &str,
    signal_threshold: f64,
    lookback_window_ms: u64,
    max_spread_bps: f64,
    strategy_enabled: bool,
) -> bytes::Bytes {
    use flatbuffers::FlatBufferBuilder;

    let mut fbb = FlatBufferBuilder::new();

    // Build StrategyParams nested table (must be built before referencing vectors).
    let product_id_off = fbb.create_string(product_id);
    let strat_start = fbb.start_table();
    fbb.push_slot_always::<flatbuffers::WIPOffset<_>>(4, product_id_off);
    fbb.push_slot::<f64>(6, signal_threshold, 0.0);
    fbb.push_slot::<u64>(8, lookback_window_ms, 0);
    fbb.push_slot::<f64>(10, max_spread_bps, 0.0);
    fbb.push_slot::<bool>(12, strategy_enabled, true);
    let strategy = fbb.end_table(strat_start);

    // Build strategies vector.
    let strategies_vec = fbb.create_vector(&[strategy]);

    // Build RiskParams nested table.
    let risk_start = fbb.start_table();
    fbb.push_slot::<f64>(4, max_position_usd, 0.0);
    fbb.push_slot::<f64>(6, max_order_size_usd, 0.0);
    fbb.push_slot::<f64>(8, max_daily_loss_usd, 0.0);
    fbb.push_slot::<u32>(10, max_orders_per_second, 0);
    fbb.push_slot::<f64>(12, max_notional_per_minute, 0.0);
    fbb.push_slot::<bool>(14, risk_enabled, true);
    let risk = fbb.end_table(risk_start);

    // Build TradingConfig root table.
    let root_start = fbb.start_table();
    fbb.push_slot::<u32>(4, schema_version, 0);
    fbb.push_slot_always::<flatbuffers::WIPOffset<flatbuffers::TableFinishedWIPOffset>>(6, risk);
    fbb.push_slot_always::<flatbuffers::WIPOffset<_>>(8, strategies_vec);
    let root = fbb.end_table(root_start);
    fbb.finish(root, None);

    bytes::Bytes::copy_from_slice(fbb.finished_data())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trading_config_round_trip() {
        let buf = build_trading_config(
            3,
            100_000.0,
            5_000.0,
            2_000.0,
            100,
            500_000.0,
            true,
            "BTC-USDT",
            0.75,
            60_000,
            25.0,
            true,
        );
        let snap = unsafe {
            TradingConfigSnapshot::from_flatbuffers(&buf, "rv-001".to_owned())
        }
        .expect("decode must succeed");

        assert_eq!(snap.schema_version, 3);
        assert_eq!(snap.risk.max_position_usd, 100_000.0);
        assert_eq!(snap.risk.max_order_size_usd, 5_000.0);
        assert_eq!(snap.risk.max_daily_loss_usd, 2_000.0);
        assert_eq!(snap.risk.max_orders_per_second, 100);
        assert_eq!(snap.risk.max_notional_per_minute, 500_000.0);
        assert!(snap.risk.enabled);
        assert_eq!(snap.strategies.len(), 1);
        assert_eq!(snap.strategies[0].product_id, "BTC-USDT");
        assert_eq!(snap.strategies[0].signal_threshold, 0.75);
        assert_eq!(snap.strategies[0].lookback_window_ms, 60_000);
        assert_eq!(snap.strategies[0].max_spread_bps, 25.0);
        assert!(snap.strategies[0].enabled);
        assert_eq!(snap.resource_version, "rv-001");
    }

    #[test]
    fn trading_config_defaults_when_no_nested() {
        use flatbuffers::FlatBufferBuilder;

        let mut fbb = FlatBufferBuilder::new();
        let root_start = fbb.start_table();
        fbb.push_slot::<u32>(4, 7, 0);
        let root = fbb.end_table(root_start);
        fbb.finish(root, None);
        let buf = bytes::Bytes::copy_from_slice(fbb.finished_data());

        let snap = unsafe { TradingConfigSnapshot::from_flatbuffers(&buf, String::new()) }
            .expect("decode must succeed with missing nested tables");

        assert_eq!(snap.schema_version, 7);
        assert_eq!(snap.risk, RiskParamsSnapshot::default());
        assert!(snap.strategies.is_empty());
    }

    #[test]
    fn empty_payload_returns_error() {
        let result = unsafe { TradingConfigSnapshot::from_flatbuffers(&[], String::new()) };
        assert!(matches!(result, Err(ParseError::EmptyPayload)));
    }
}
