pub use controlla_runtime::{
    apps, artifacts, browser_workflow, cache, canva, capability, capability_router, capcut_recipe,
    doctor, http_auth, http_server, jobs, metrics, native_host, native_setup, runtime_observer,
    semantic_state, skill_runtime, skill_store, skills, slides_deck, verifier, workflow,
};

#[allow(dead_code, clippy::unnecessary_sort_by)]
mod legacy_mcp {
    include!("../mcp.rs");

    mod v2 {
        include!("../mcp/v2.rs");

        pub(super) mod full {
            include!("../mcp/v2_full.rs");
        }
    }

    pub(crate) fn run_compact() -> Result<(), Box<dyn std::error::Error>> {
        v2::full::run()
    }
}

fn main() {
    if let Err(error) = legacy_mcp::run_compact() {
        eprintln!("Controlla v2 MCP server failed: {error}");
        std::process::exit(1);
    }
}
