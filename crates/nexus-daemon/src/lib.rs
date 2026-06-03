//! Kaiju daemon library.
//!
//! Exposes the HTTP API, agent store, tmux integration, and the background
//! monitor as a library so they can be exercised by integration tests. The
//! `nexus-daemon` binary is a thin wrapper around [`server::run`].

pub mod api;
pub mod auth;
pub mod batch;
pub mod dashboard;
pub mod monitor;
pub mod notify;
pub mod persist;
pub mod reconcile;
pub mod scheduler;
pub mod server;
pub mod store;
pub mod task_store;
pub mod tmux;
pub mod worktree;
