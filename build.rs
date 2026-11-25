use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let target = env::var("TARGET").expect("Cargo target");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    let script = out.join("paperroom-version.rc");
    fs::write(&script, version_resource()).expect("write Windows version resource");

    let (mut compiler, object) = if let Some(arch) = target.strip_suffix("-pc-windows-gnu") {
        let object = out.join("paperroom-version.o");
        let mut compiler = Command::new(format!("{arch}-w64-mingw32-windres"));
        compiler
            .args(["-O", "coff", "-i"])
            .arg(&script)
            .arg("-o")
            .arg(&object);
        (compiler, object)
    } else if target.ends_with("-pc-windows-msvc") {
        let object = out.join("paperroom-version.res");
        let mut compiler =
            cc::windows_registry::find(&target, "rc.exe").unwrap_or_else(|| Command::new("rc.exe"));
        compiler
            .args(["/nologo", "/fo"])
            .arg(&object)
            .arg(&script);
        (compiler, object)
    } else {
        panic!("Windows version resource compilation is unsupported for {target}");
    };

    let status = compiler
        .status()
        .unwrap_or_else(|error| panic!("could not run {compiler:?}: {error}"));
    assert!(status.success(), "Windows resource compiler failed: {compiler:?}");
    println!("cargo:rustc-link-arg-bin=paperroom={}", object.display());
}

fn version_resource() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let [major, minor, patch] = [
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
    ]
    .map(|part| part.parse::<u16>().expect("Windows version component must fit in 16 bits"));
    let prerelease = if env!("CARGO_PKG_VERSION_PRE").is_empty() {
        0
    } else {
        2
    };
    format!(
        r#"
LANGUAGE 9, 1
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS {prerelease}L
FILEOS 0x00040004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "Paperroom\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "Paperroom\0"
      VALUE "OriginalFilename", "Paperroom.exe\0"
      VALUE "ProductName", "Paperroom\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#
    )
}
