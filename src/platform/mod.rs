//! `evnx platform`: two-way sync with a hosting platform's environment
//! variables. Vercel is the first; others (Netlify, Fly, Railway, Render) slot in
//! as another `--to` value and another module beside `vercel`.
//!
//! Phase 1 is read-only against the platform: `diff` and `pull`. Behind the `net`
//! feature, like everything that opens a socket.
//!
//! # Never
//!
//! Print a value. Write a value into a log or the state file. Keep the token
//! anywhere but the one request header.

pub mod error;
pub mod secret;
pub mod vercel;

pub use error::exit_code_of;
