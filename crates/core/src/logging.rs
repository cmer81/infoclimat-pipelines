//! Init standard de `tracing_subscriber` pour toutes les CLI du workspace.

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Initialise le logger. Format JSON si la var `LOG_FORMAT=json`, sinon
/// human-readable. Filtre via `RUST_LOG` (default: `info`).
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(filter);

    if std::env::var("LOG_FORMAT").as_deref() == Ok("json") {
        registry.with(fmt::layer().json()).init();
    } else {
        registry.with(fmt::layer()).init();
    }
}
