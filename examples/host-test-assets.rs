// Writes the shared host assets tests import. Test files run concurrently and
// each runs this example, so a file is replaced only when its bytes differ and
// always through a rename: a reader never sees a truncated or partial asset.
fn put(root: &std::path::Path, name: &str, bytes: &[u8]) {
    let path = root.join(name);
    if std::fs::read(&path).is_ok_and(|existing| existing == bytes) {
        return;
    }
    let temporary = root.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes).unwrap();
    std::fs::rename(&temporary, &path).unwrap();
}

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("hosts");
    put(&root, "ink-abi.mjs", ink_abi::PACKED_JS_SOURCE.as_bytes());
    put(&root, "ink-selection.mjs", ink_abi::SELECTION_JS_SOURCE.as_bytes());
    put(&root, "ink-selection.wasm", ink_abi::SELECTION_WASM);
}
