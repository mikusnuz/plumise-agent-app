fn main() {
    // Copy dylibs to target dir for dev mode (llama-server needs them at @rpath)
    let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
    if !out_dir.is_empty() {
        // OUT_DIR is like target/debug/build/<crate>/out — go up to target/debug/
        let target_dir = std::path::Path::new(&out_dir)
            .ancestors()
            .nth(3)
            .unwrap_or(std::path::Path::new("."));
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        if let Ok(entries) = std::fs::read_dir(src_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("lib") && name_str.ends_with(".dylib") {
                    let dest = target_dir.join(&name);
                    let _ = std::fs::copy(entry.path(), &dest);
                }
            }
        }
    }
    tauri_build::build()
}
