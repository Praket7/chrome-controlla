pub use controlla_runtime::{
    apps, artifacts, browser_workflow, cache, canva, capability, capability_router, capcut_recipe,
    doctor, http_auth, http_server, jobs, metrics, native_host, native_setup, runtime_observer,
    semantic_state, skill_runtime, skill_store, skills, slides_deck, v3, verifier, workflow,
};

#[allow(dead_code, clippy::unnecessary_sort_by)]
mod legacy_mcp {
    include!("../mcp.rs");

    mod v2 {
        include!("../mcp/v2.rs");

        pub(super) mod full {
            include!("../mcp/v2_full.rs");
        }

        pub(super) mod v3 {
            include!("../mcp/v3_full.rs");
        }

        pub(super) async fn run_v3() -> Result<(), String> {
            v3::run().await
        }
    }

    pub(crate) async fn run_compact() -> Result<(), String> {
        v2::run_v3().await
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = legacy_mcp::run_compact().await {
        eprintln!("Controlla v3 MCP server failed: {error}");
        std::process::exit(1);
    }
}
