fn main() {
    for file in [
        "resources/app.rc",
        "resources/app.ico",
        "resources/app.manifest",
        "resources/THIRD-PARTY-NOTICES.txt",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }
    embed_resource::compile("resources/app.rc", embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
