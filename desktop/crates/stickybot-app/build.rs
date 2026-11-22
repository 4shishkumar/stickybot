fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/volume.m");
        cc::Build::new().file("src/volume.m").flag("-fobjc-arc").flag("-fblocks").compile("stickybot_volume");
    }
    tauri_build::build()
}
