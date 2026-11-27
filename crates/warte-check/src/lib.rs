//! warte's validators for the reports other tools hand it: `architecture.v1`
//! (from spar) and `temper-report.v1` (from temper). Pure functions over bytes,
//! with no async runtime and no I/O, so the same code runs inside `warte serve`
//! and as a WASI p2 component (`warte-check`) on wasmtime or kiln.

pub mod architecture;
pub mod temper_report;
