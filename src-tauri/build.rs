fn main() {
    println!("cargo:rerun-if-changed=native.m");
    cc::Build::new()
        .file("native.m")
        .flag("-fobjc-arc")
        .flag("-Wno-incompatible-pointer-types")
        .compile("sumi_native");
    for name in [
        "Foundation",
        "Vision",
        "ImageIO",
        "AppKit",
        "PDFKit",
        "NaturalLanguage",
        "ApplicationServices",
        "IOKit",
        "UserNotifications",
    ] {
        println!("cargo:rustc-link-lib=framework={name}");
    }
    tauri_build::build()
}
