use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, env, fs, path::PathBuf};

const EXTENSION_RUNTIME_ASSETS: &[&str] = &[
    "background.js",
    "manifest.json",
    "open-recovery.html",
    "popup.html",
    "popup.js",
    "secure-input.css",
    "secure-input.html",
    "secure-input.js",
    "lib/transport.js",
    "lib/frame-tree.js",
    "lib/debugger-sessions.js",
    "lib/presentation-input.js",
    "lib/input-scripts.js",
    "lib/typing-support.js",
    "lib/input.js",
    "lib/editor-adapters.js",
    "lib/observation.js",
    "lib/pointer.js",
    "lib/resource-keys.js",
    "lib/owned-resources.js",
    "lib/presentation-slots.js",
    "lib/tab-control.js",
    "lib/focus-emulation.js",
    "lib/owned-pool.js",
    "lib/execution-counters.js",
    "lib/navigation-url.js",
    "lib/frame-multitouch.js",
    "lib/owned-open.js",
];

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let extension_dir = manifest_dir.join("../../extensions/chrome-controlla");
    let runtime_asset_names = EXTENSION_RUNTIME_ASSETS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let allowed_non_runtime_files = [
        "README.md",
        "test-background.cjs",
        "test-observation.mjs",
        "test-p0-resources.mjs",
        "test-v3-batch.cjs",
        "lib",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let actual_files = fs::read_dir(&extension_dir)
        .expect("read extension directory")
        .map(|entry| {
            entry
                .expect("read extension directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<BTreeSet<_>>();
    let expected_files = runtime_asset_names
        .iter()
        .map(|name| name.split('/').next().unwrap_or(name).to_owned())
        .chain(
            allowed_non_runtime_files
                .iter()
                .map(|name| (*name).to_owned()),
        )
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_files, expected_files,
        "classify new extension files as runtime assets or test/docs files"
    );

    let mut assets = Vec::with_capacity(EXTENSION_RUNTIME_ASSETS.len());
    for name in EXTENSION_RUNTIME_ASSETS {
        let path = extension_dir.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        assets.push((*name, fs::read(path).expect("read extension runtime asset")));
    }
    let manifest = assets
        .iter()
        .find(|(name, _)| *name == "manifest.json")
        .expect("manifest is a runtime asset")
        .1
        .as_slice();
    let manifest_json: serde_json::Value =
        serde_json::from_slice(manifest).expect("parse extension manifest");
    let runtime_version = env::var("CARGO_PKG_VERSION").expect("package version");
    assert_eq!(
        manifest_json["version"].as_str(),
        Some(runtime_version.as_str()),
        "extension and runtime versions must match"
    );

    let mut hasher = Sha256::new();
    for (name, bytes) in &assets {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        hasher.update([0]);
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    // Persisted timing cohorts must not survive changes to the executing
    // runtime, extension, dependency graph, or target platform unnoticed.
    let mut source_files = Vec::new();
    fn sources(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("read source directory") {
            let path = entry.expect("read source entry").path();
            if path.is_dir() {
                sources(&path, files);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "rs" || ext == "js")
            {
                files.push(path);
            }
        }
    }
    sources(&manifest_dir.join("src"), &mut source_files);
    sources(
        &manifest_dir.join("../controlla-browser/src"),
        &mut source_files,
    );
    source_files.extend([
        manifest_dir.join("build.rs"),
        manifest_dir.join("../../Cargo.lock"),
        manifest_dir.join("../../rust-toolchain.toml"),
    ]);
    source_files.sort();
    let mut cost_hasher = Sha256::new();
    cost_hasher.update(digest.as_bytes());
    cost_hasher.update(env::var("TARGET").expect("target triple").as_bytes());
    for key in ["PROFILE", "OPT_LEVEL", "DEBUG", "CARGO_ENCODED_RUSTFLAGS"] {
        println!("cargo:rerun-if-env-changed={key}");
        cost_hasher.update(key.as_bytes());
        cost_hasher.update([0]);
        cost_hasher.update(env::var(key).unwrap_or_default().as_bytes());
        cost_hasher.update([0]);
    }
    for path in source_files {
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path).expect("read cost cohort source");
        cost_hasher.update((bytes.len() as u64).to_be_bytes());
        cost_hasher.update(bytes);
    }
    let cost_scope = cost_hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    println!("cargo:rustc-env=CONTROLLA_COST_BUILD_ID={cost_scope}");
    println!(
        "cargo:rustc-env=CONTROLLA_EXTENSION_BUILD_ID=controlla-chrome-extension:{runtime_version}:{digest}"
    );
}
