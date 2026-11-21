fn main() {
    #[cfg(all(target_os = "windows", feature = "win-icon"))]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.compile().expect("Failed to compile Windows resources");
    }

    // The static FFmpeg from vcpkg (feature `video`, Windows MSVC) carries the
    // Media Foundation codecs, whose interface IDs live in these system
    // libraries; ffmpeg-sys-next does not ask for them.
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if msvc && std::env::var_os("CARGO_FEATURE_VIDEO").is_some() {
        for lib in ["mfuuid", "strmiids", "mfplat", "ole32"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
}
