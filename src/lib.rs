//! warte — the PulseEngine control tower.
//!
//! A CI dashboard with an ask feature, agent orchestration, access control,
//! multi-instance sync, and mobile access.
//!
//! German: "watchtower" — and "bitte warte auf mich", "please wait for me".

pub mod access;
pub mod admission;
pub mod agent;
pub mod alerts;
pub mod artifact;
pub mod auth;
pub mod briefing;
pub mod ca;
pub mod cli;
pub mod config;
pub mod contract;
pub mod costs;
pub mod critpath;
pub mod dashboard;
pub mod db;
pub mod discovery;
pub mod docs;
pub mod fleet;
pub mod ghsource;
pub mod history;
pub mod hub;
pub mod jobs;
pub mod migrate;
pub mod platform;
pub mod probe;
pub mod realtime;
pub mod rivetext;
pub mod server;
pub mod sigil;
pub mod skills;
pub mod status;
pub mod sync;
pub mod temper;
pub mod triage;
pub mod watch;

pub use config::Config;
