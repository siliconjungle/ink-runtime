fn main() {
    println!("cargo:rerun-if-changed=program.c");
    for name in ["INK_CC", "INK_CC_KIND", "INK_AR"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let compiler = std::env::var("INK_CC").ok();
    let mut build = cc::Build::new();
    build
        .file("program.c")
        .opt_level(2)
        .flag_if_supported("-std=c11")
        .flag_if_supported("-ffp-contract=off");
    if std::env::var("TARGET").is_ok_and(|t| t == "wasm32-unknown-unknown") {
        if let Some(compiler) = compiler {
            build.compiler(compiler);
        } else {
            build.compiler("clang");
        }
        build
            .no_default_flags(true)
            .flag("-O2")
            .flag("-ffreestanding");
        if std::env::var("INK_CC_KIND").is_ok_and(|v| v == "zig") {
            build.flag("-target").flag("wasm32-freestanding");
        } else {
            build.flag("--target=wasm32-unknown-unknown");
        }
        if let Ok(ar) = std::env::var("INK_AR") {
            build.archiver(ar);
        }
    } else if let Some(compiler) = compiler {
        build.compiler(compiler);
    }
    build.compile("ink_program");
}
