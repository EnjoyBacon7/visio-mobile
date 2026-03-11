fn main() {
    // On macOS (APFS), fs::copy uses clonefile() which fails with EEXIST if
    // the destination already exists. Tauri-build doesn't remove stale resource
    // copies before re-copying, so we clean them up here.
    if let Ok(out_dir) = std::env::var("OUT_DIR") {
        let target_dir = std::path::Path::new(&out_dir)
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent());
        if let Some(target_dir) = target_dir {
            for dir in ["i18n", "backgrounds", "models"] {
                let _ = std::fs::remove_dir_all(target_dir.join(dir));
            }
        }
    }

    tauri_build::build();
}
