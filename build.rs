#[path = "src/icon.rs"]
mod icon;

fn main() {
    slint_build::compile_with_config(
        "ui/main.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .expect("failed to compile ui/main.slint");

    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=assets/downcop.manifest");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

#[cfg(windows)]
fn windows_resources() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("downcop.ico");
    std::fs::write(&ico, icon::encode_ico(&[16, 20, 24, 32, 40, 48, 64, 256], icon::IconStyle::App))
        .expect("write icon");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap())
        .set_manifest_file("assets/downcop.manifest")
        .set("FileDescription", "downcop - Download Cop")
        .set("ProductName", "downcop (다운로드단속반)");
    res.compile().expect("failed to embed Windows resources");
}

// Cross-compiling from a non-Windows host: winresource is only a dependency
// on Windows hosts, so embed resources via the mingw toolchain directly.
#[cfg(not(windows))]
fn windows_resources() {
    use std::process::Command;
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("downcop.ico");
    std::fs::write(&ico, icon::encode_ico(&[16, 20, 24, 32, 40, 48, 64, 256], icon::IconStyle::App))
        .expect("write icon");
    let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets/downcop.manifest");
    let rc = out.join("downcop.rc");
    std::fs::write(
        &rc,
        format!(
            "1 ICON \"{}\"\n1 24 \"{}\"\n",
            ico.display().to_string().replace('\\', "/"),
            manifest.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into());
    let obj = out.join("downcop_res.o");
    let status = Command::new(&windres).arg(&rc).arg("-O").arg("coff").arg("-o").arg(&obj).status();
    match status {
        Ok(s) if s.success() => {
            // link the object directly into the binary
            println!("cargo:rustc-link-arg-bins={}", obj.display());
        }
        _ => println!("cargo:warning=windres not found; building without icon/manifest"),
    }
}
