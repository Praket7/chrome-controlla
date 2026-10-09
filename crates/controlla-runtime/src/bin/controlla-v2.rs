pub use controlla_runtime::{
    apps, artifacts, cache, canva, capability, capcut_recipe, doctor, http_auth, http_server,
    jobs, native_host, native_setup, slides_deck, verifier, workflow,
};

mod legacy_mcp {
    include!("../mcp.rs");

    mod v2 {
        include!("../mcp/v2.rs");
    }

    pub(crate) fn run_compact() -> Result<(), Box<dyn std::error::Error>> {
        v2::run()
    }
}

fn main() {
    if let Err(error) = legacy_mcp::run_compact() {
        eprintln!("Controlla v2 MCP server failed: {error}");
        std::process::exit(1);
    }
}
