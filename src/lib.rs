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
pub mod cli;
pub mod config;
pub mod db;
pub mod discovery;
pub mod docs;
pub mod migrate;
pub mod platform;
pub mod server;
pub mod sync;

pub use config::Config;
