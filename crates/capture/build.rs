use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn static_lib_name(package: &str) -> &str {
    package.strip_prefix("lib").unwrap_or(package)
}

fn vcpkg_triplet() -> String {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_else(|_| "macos".to_owned());
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".to_owned());

    if target_os != "macos" {
        panic!("capture build.rs only supports macOS targets");
    }

    match target_arch.as_str() {
        "aarch64" => "arm64-osx".to_owned(),
        "x86_64" => "x64-osx".to_owned(),
        arch => format!("{arch}-osx"),
    }
}

fn link_vcpkg(vcpkg_root: PathBuf, package: &str) -> PathBuf {
    let installed_root = env::var_os("VCPKG_INSTALLED_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| vcpkg_root.join("installed"));
    let package_root = installed_root.join(vcpkg_triplet());

    println!("cargo:rustc-link-lib=static={}", static_lib_name(package));
    println!(
        "cargo:rustc-link-search={}",
        package_root.join("lib").display()
    );

    package_root.join("include")
}

fn link_homebrew_m1(package: &str) -> PathBuf {
    let cellar = PathBuf::from("/opt/homebrew/Cellar").join(package);
    let entries = fs::read_dir(&cellar).unwrap_or_else(|_| {
        panic!(
            "could not find {package} in {}; install it with your declared package manager first",
            cellar.display()
        )
    });

    let mut versions = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    versions.sort_unstable();

    let package_root = versions.pop().unwrap_or_else(|| {
        panic!(
            "no installed versions of {package} found in {}",
            cellar.display()
        )
    });

    println!("cargo:rustc-link-lib=static={}", static_lib_name(package));
    println!(
        "cargo:rustc-link-search={}",
        package_root.join("lib").display()
    );

    package_root.join("include")
}

fn link_nix(package: &str) -> Option<PathBuf> {
    // Check for explicit env var first: LIBVPX_PATH or LIBYUV_PATH
    let env_name = format!("{}_PATH", package.to_uppercase().replace('-', "_"));
    println!("cargo:rerun-if-env-changed={env_name}");
    if let Ok(path) = env::var(&env_name) {
        let p = PathBuf::from(path);
        if p.join("lib").exists() && p.join("include").exists() {
            println!("cargo:rustc-link-lib=static={}", static_lib_name(package));
            println!("cargo:rustc-link-search={}", p.join("lib").display());
            return Some(p.join("include"));
        }
    }

    let short_name = static_lib_name(package);
    if let Ok(lib) =
        pkg_config::probe_library(short_name).or_else(|_| pkg_config::probe_library(package))
    {
        return Some(lib.include_paths.into_iter().next().unwrap_or_default());
    }

    None
}

fn find_package(package: &str) -> Vec<PathBuf> {
    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");
    println!("cargo:rerun-if-env-changed=VCPKG_INSTALLED_ROOT");

    // 1. Try nix / pkg-config
    if let Some(include) = link_nix(package) {
        return vec![include];
    }
    // 2. Try vcpkg
    if let Ok(vcpkg_root) = env::var("VCPKG_ROOT") {
        return vec![link_vcpkg(PathBuf::from(vcpkg_root), package)];
    }
    // 3. Fallback to homebrew
    vec![link_homebrew_m1(package)]
}

fn generate_bindings(header: &Path, include_paths: &[PathBuf], output: &Path, regex: &str) {
    let mut builder = bindgen::builder()
        .header(header.display().to_string())
        .allowlist_type(regex)
        .allowlist_var(regex)
        .allowlist_function(regex)
        .rustified_enum(regex)
        .trust_clang_mangling(false)
        .layout_tests(false)
        .generate_comments(false);

    for include_path in include_paths {
        builder = builder.clang_arg(format!("-I{}", include_path.display()));
    }

    builder
        .generate()
        .unwrap_or_else(|err| {
            panic!(
                "failed to generate bindings for {}: {err}",
                header.display()
            )
        })
        .write_to_file(output)
        .unwrap_or_else(|err| panic!("failed to write bindings to {}: {err}", output.display()));
}

fn gen_package(package: &str, header_name: &str, output_name: &str, regex: &str) {
    let include_paths = find_package(package);
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap_or_default());
    let header = manifest_dir.join("src").join("bindings").join(header_name);
    let output = out_dir.join(output_name);

    println!("cargo:rerun-if-changed={}", header.display());
    for include_path in &include_paths {
        println!("cargo:rerun-if-changed={}", include_path.display());
    }

    generate_bindings(&header, &include_paths, &output, regex);
}

fn main() {
    println!("cargo:rustc-cfg=quartz");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
    println!("cargo:rustc-link-lib=framework=IOSurface");
    println!("cargo:rustc-link-lib=framework=ApplicationServices");

    gen_package("libvpx", "vpx_ffi.h", "vpx_ffi.rs", "^[vV].*");
    gen_package("libyuv", "yuv_ffi.h", "yuv_ffi.rs", ".*");
}
