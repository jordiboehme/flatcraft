//! System font database, loaded once.

use std::sync::{Arc, OnceLock};

static SYSTEM_FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();

/// Returns the shared system font database. Fonts are loaded on first use.
pub fn system_fonts() -> Arc<usvg::fontdb::Database> {
    SYSTEM_FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}
