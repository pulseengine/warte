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
pub mod canary;
pub mod cihealth;
pub mod cli;
pub mod config;
pub mod contract;
pub mod costs;
pub mod critpath;
pub mod dashboard;
pub mod db;
pub mod discovery;
pub mod docs;
pub mod expiry;
pub mod fleet;
pub mod forecast;
pub mod ghsource;
pub mod history;
pub mod hub;
pub mod incidents;
pub mod jobs;
pub mod login;
pub mod migrate;
pub mod nav;
pub mod news;
pub mod operator;
pub mod platform;
pub mod probe;
pub mod realtime;
pub mod rivetext;
pub mod server;
pub mod sessions;
pub mod sigil;
pub mod skills;
pub mod status;
pub mod sync;
pub mod temper;
pub mod triage;
pub mod watch;

pub use config::Config;
