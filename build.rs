fn main() {
    for file in [
        "app.rc",
        "app.ico",
        "app.manifest",
        "THIRD-PARTY-NOTICES.txt",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }
    embed_resource::compile("app.rc", embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
