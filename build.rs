fn main() {
    println!("cargo:rerun-if-changed=packaging/nivumi.rc");
    println!("cargo:rerun-if-changed=assets/brand/app.ico");
    println!("cargo:rerun-if-changed=assets/brand/tray.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let rc = std::fs::read_to_string(root.join("packaging/nivumi.rc"))
        .unwrap()
        .replace(
            "@VERSION_COMMA@",
            &format!("{},0", version.replace('.', ",")),
        )
        .replace("@VERSION@", &version)
        .replace(
            "@APP_ICON@",
            &root
                .join("assets/brand/app.ico")
                .to_string_lossy()
                .replace('\\', "/"),
        )
        .replace(
            "@TRAY_ICON@",
            &root
                .join("assets/brand/tray.ico")
                .to_string_lossy()
                .replace('\\', "/"),
        );
    let resource = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("nivumi.rc");
    std::fs::write(&resource, rc).unwrap();
    embed_resource::compile_for_everything(resource, embed_resource::NONE)
        .manifest_required()
        .expect("Nivumi requires Windows icon and version resources");
}
