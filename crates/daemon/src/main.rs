//! `shellmind-daemon` – the resident helper process.
//!
//! It listens on a local Unix socket and serves the same completion /
//! history queries the CLI would answer, but from a warm in-process cache.
//! It also performs incremental history indexing and backfills embeddings
//! in the background so the first interactive query is already fast.
//!
//! The daemon never talks to the network on its own initiative; AI calls
//! only happen while serving an explicit user-triggered request.

fn main() {
    shellmind_core::daemon::server::run_server();
}
