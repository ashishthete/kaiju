//! Kaiju daemon library.
//!
//! Exposes the HTTP API, agent store, tmux integration, and the background
//! monitor as a library so they can be exercised by integration tests. The
//! `kaiju-daemon` binary is a thin wrapper around [`server::run`].

pub mod api;
pub mod auth;
pub mod batch;
pub mod dashboard;
pub mod devices;
pub mod files;
pub mod judge;
pub mod logstore;
pub mod monitor;
pub mod net;
pub mod notify;
pub mod pair_api;
pub mod pairing;
pub mod persist;
pub mod reconcile;
pub mod scheduler;
pub mod server;
pub mod settings;
pub mod store;
pub mod task_store;
pub mod terminal;
pub mod tmux;
pub mod worktree;
