fn main() {
    let parser_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/parser.c");

    if !parser_path.exists() {
        // Grammar not yet compiled. Emit a tiny C stub that exports
        // `tree_sitter_keyflow` returning NULL so the crate links and
        // unit tests pass; running `tree-sitter generate` from the
        // grammar root produces a real `src/parser.c` that overrides
        // this stub. Downstream code that calls `LANGUAGE.into()` will
        // get a NULL `Language`, which `tree-sitter` rejects with a
        // descriptive error rather than crashing.
        println!(
            "cargo:warning=tree-sitter-keyflow: src/parser.c missing — run `tree-sitter generate` to build the C parser. Compiling a NULL stub."
        );
        let stub_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let stub = stub_dir.join("parser_stub.c");
        std::fs::write(
            &stub,
            "void *tree_sitter_keyflow(void) { return (void*)0; }\n",
        )
        .expect("write parser stub");
        cc::Build::new().file(&stub).compile("tree-sitter-keyflow");
        return;
    }

    let mut build = cc::Build::new();
    build
        .include(parser_path.parent().unwrap())
        .file(&parser_path)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .flag_if_supported("-Wno-trigraphs");

    let scanner = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/scanner.c");
    if scanner.exists() {
        build.file(scanner);
    }

    build.compile("tree-sitter-keyflow");
    println!("cargo:rerun-if-changed=src/parser.c");
    println!("cargo:rerun-if-changed=src/scanner.c");
}
