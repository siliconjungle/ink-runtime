fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("hosts");
    for (n, s) in [
        ("ink-abi.mjs", ink_abi::PACKED_JS_SOURCE),
        ("ink-selection.mjs", ink_abi::SELECTION_JS_SOURCE),
    ] {
        std::fs::write(root.join(n), s).unwrap();
    }
    std::fs::write(root.join("ink-selection.wasm"), ink_abi::SELECTION_WASM).unwrap();
}
