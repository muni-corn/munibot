use std::sync::Arc;

use munibot_core::db::DbPool;

use crate::{
    memory::{
        DieselMemoryOptIn, DieselMemoryStore, GatedMemoryStore, MemoryStore, MemoryToolBackend,
    },
    tools::{ForgetTool, RememberTool, ToolRegistry},
};

/// Registers the `remember`/`forget` tools into `registry`, backed by a
/// diesel-backed, opt-in-gated [`MemoryStore`] over `pool`.
///
/// Lives here, in `memory`, rather than as a method on
/// [`crate::tools::ToolRegistry`] itself: constructing the gated diesel
/// store needs types from this module, and `tools` sits below `memory` in
/// this crate's dependency graph, so `tools` reaching back up here would be
/// the wrong-direction dependency `crate::tools::MemoryBackend`'s own doc
/// comment already explains. `memory` already depends on `tools`, so the
/// wiring belongs on this side.
///
/// Separate from [`crate::tools::ToolRegistry::from_env`] because these
/// tools need a database connection pool, not just environment variables -
/// `from_env` has no way to obtain one, and forcing every caller (including
/// every test using a tool-only, in-memory fixture) to supply a pool it does
/// not need would be a worse default than a second, explicit call.
pub fn register_memory_tools(registry: &mut ToolRegistry, pool: DbPool) {
    let store: Arc<dyn MemoryStore> = Arc::new(GatedMemoryStore::new(
        DieselMemoryStore::new(pool.clone()),
        DieselMemoryOptIn::new(pool),
    ));
    let backend = Arc::new(MemoryToolBackend::new(store));

    registry.register(Arc::new(RememberTool::new(backend.clone())));
    registry.register(Arc::new(ForgetTool::new(backend)));
}
