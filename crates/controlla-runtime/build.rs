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
];

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let extension_dir = manifest_dir.join("../../extensions/chrome-controlla");
    let runtime_asset_names = EXTENSION_RUNTIME_ASSETS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let allowed_non_runtime_files = ["README.md", "test-background.cjs", "test-v3-batch.cjs"]
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
        .map(|name| (*name).to_owned())
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
    println!(
        "cargo:rustc-env=CONTROLLA_EXTENSION_BUILD_ID=controlla-chrome-extension:{runtime_version}:{digest}"
    );
}
