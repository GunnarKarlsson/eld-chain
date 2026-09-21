//! Re-exports of `eld_common` log sanitizers for node/CLI callers.
//!
//! Process-wide tracing setup lives in the `eld` CLI, faucet, and node binaries.

pub use eld_common::logging::{LogSanitizer, SanitizedLog, SanitizedLoggable};
