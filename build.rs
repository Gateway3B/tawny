//! Embed the release's definitions for offline recovery, without data repairs.
fn main() {
    println!("cargo:rerun-if-changed=database/schema");
    if std::env::var_os("CARGO_FEATURE_SERVER").is_none() {
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let mut files = std::fs::read_dir(root.join("database/schema"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "surql")
        })
        .collect::<Vec<_>>();
    files.sort();
    let entries = files
        .iter()
        .map(|file| format!("include_str!({:?}),", file.to_str().unwrap()))
        .collect::<String>();
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        output.join("privacy_schema.rs"),
        format!("const RESTORE_SCHEMA: &[&str] = &[{entries}];"),
    )
    .unwrap();
}
