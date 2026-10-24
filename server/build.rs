use std::{env, path::PathBuf, process::Command};

fn output(command: &mut Command) -> std::process::Output {
    let output = command
        .output()
        .expect("Не вдалося запустити Xcode Swift compiler");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_APPLE");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || env::var_os("CARGO_FEATURE_APPLE").is_none()
    {
        return;
    }
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("Непідтримувана архітектура: {other}"),
    };
    let target = format!("{arch}-apple-macos26.0");
    let native = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../native");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut compiler = Command::new("xcrun");
    compiler
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-O",
            "-module-name",
            "CommentTranslateNative",
            "-target",
            &target,
            "-module-cache-path",
        ])
        .arg(out.join("swift-module-cache"));
    for source in [
        "TranslationStrategy.swift",
        "TranslationService.swift",
        "AppleTranslate.swift",
        "Bridge.swift",
    ] {
        let path = native.join(source);
        println!("cargo:rerun-if-changed={}", path.display());
        compiler.arg(path);
    }
    compiler
        .arg("-o")
        .arg(out.join("libCommentTranslateNative.a"));
    output(&mut compiler);
    let info =
        output(Command::new("xcrun").args(["swiftc", "-print-target-info", "-target", &target]));
    let info: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    for path in info["paths"]["runtimeLibraryPaths"].as_array().unwrap() {
        println!("cargo:rustc-link-search=native={}", path.as_str().unwrap());
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=CommentTranslateNative");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=NaturalLanguage");
    println!("cargo:rustc-link-lib=framework=Translation");
    println!("cargo:rustc-link-arg=-mmacosx-version-min=26.0");
}
