fn main() {
    println!("cargo:rerun-if-changed=src/db/storage_fault_vfs.c");
    if std::env::var_os("CARGO_FEATURE_STORAGE_FAULT_INJECTION").is_some() {
        let sqlite_include = std::env::var_os("DEP_SQLITE3_INCLUDE")
            .expect("libsqlite3-sys must expose its matching bundled headers");
        cc::Build::new()
            .file("src/db/storage_fault_vfs.c")
            .include(sqlite_include)
            .flag_if_supported("-std=c11")
            .warnings(true)
            .extra_warnings(true)
            .warnings_into_errors(true)
            .compile("concord_storage_fault_vfs");
    }
}
