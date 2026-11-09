//! Vercel: the REST client, token and project discovery that `evnx platform
//! diff|pull --to vercel` stand on.
//!
//! Vercel has two kinds of variable. A **Config** value can be read back; a
//! **Secret** can never be. Values are never printed.

pub mod api;
pub mod auth;
pub mod project;
