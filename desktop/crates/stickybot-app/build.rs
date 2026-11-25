fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/volume.m");
        cc::Build::new().file("src/volume.m").flag("-fobjc-arc").flag("-fblocks").compile("stickybot_volume");
        println!("cargo:rerun-if-changed=src/mac_voice.m");
        cc::Build::new().file("src/mac_voice.m").flag("-fobjc-arc").flag("-fblocks").compile("mac_voice");
        for framework in ["AppKit", "QuartzCore", "AVFoundation", "CoreAudio", "AudioToolbox", "CoreMedia", "Carbon"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
    tauri_build::build()
}
