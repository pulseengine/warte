//! warte — the PulseEngine control tower.
//!
//! A CI dashboard with an ask feature, agent orchestration, access control,
//! multi-instance sync, and mobile access.
//!
//! German: "watchtower" — and "bitte warte auf mich", "please wait for me".

pub mod access;
pub mod agent;
pub mod artifact;
pub mod auth;
pub mod ca;
pub mod ci;
pub mod cli;
pub mod config;
pub mod contract;
pub mod critpath;
pub mod dashboard;
pub mod db;
pub mod discovery;
pub mod docs;
pub mod eta;
pub mod migrate;
pub mod platform;
pub mod poller;
pub mod realtime;
pub mod rivetext;
pub mod server;
pub mod sigil;
pub mod status;
pub mod sync;
pub mod temper;

pub use config::Config;
