use std::{env, path::PathBuf, process::Command};

fn main() {
    // With `sherpa-shared`, the bench links libsherpa-onnx-c-api.dylib + libonnxruntime.dylib via @rpath.
    // Look next to the executable (a Tauri bundle would use @executable_path/../Frameworks instead);
    // `stt-testset/run_all.sh` copies the dylibs there from sherpa-onnx-sys's OUT_DIR.
    if env::var_os("CARGO_FEATURE_SHERPA_SHARED").is_some() {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path");
    }
    if env::var_os("CARGO_FEATURE_FLUID").is_some() {
        link_fluid_bridge();
    }
}

/// Builds swift/FluidBridge (SwiftPM, static lib wrapping FluidAudio) and links it plus the Swift runtime.
fn link_fluid_bridge() {
    let pkg = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("swift/FluidBridge");
    println!("cargo:rerun-if-changed={}", pkg.join("Sources").display());
    println!("cargo:rerun-if-changed={}", pkg.join("Package.swift").display());
    let ok = Command::new("swift")
        .args(["build", "-c", "release", "--package-path"])
        .arg(&pkg)
        .status()
        .expect("swift (Xcode command line tools) is needed for the `fluid` feature");
    assert!(ok.success(), "swift build of FluidBridge failed");
    let bin = String::from_utf8(
        Command::new("swift")
            .args(["build", "-c", "release", "--show-bin-path", "--package-path"])
            .arg(&pkg)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    println!("cargo:rustc-link-search=native={}", bin.trim());
    println!("cargo:rustc-link-lib=static=FluidBridge");
    // FluidAudio's NemoTextProcessing (inverse text normalisation) is itself a Rust staticlib
    println!("cargo:rustc-link-lib=static=text_processing_rs");
    // Swift runtime + compatibility shims: search paths from the toolchain; the frameworks and
    // -lswift* libs come in through the objects' autolink entries.
    let info = String::from_utf8(Command::new("swift").args(["-print-target-info"]).output().unwrap().stdout).unwrap();
    for line in info.lines() {
        let l = line.trim().trim_end_matches(',').trim_matches('"');
        if l.starts_with('/') && l.contains("/lib/swift") {
            println!("cargo:rustc-link-search=native={l}");
        }
    }
    let tc = String::from_utf8(Command::new("xcrun").args(["--find", "swift"]).output().unwrap().stdout).unwrap();
    let tc = PathBuf::from(tc.trim()).parent().unwrap().parent().unwrap().join("lib/swift/macosx");
    println!("cargo:rustc-link-search=native={}", tc.display());
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rustc-link-lib=c++");
    for f in ["Foundation", "CoreML", "Accelerate", "AVFoundation", "CoreAudio", "AudioToolbox", "Metal"] {
        println!("cargo:rustc-link-lib=framework={f}");
    }
}
