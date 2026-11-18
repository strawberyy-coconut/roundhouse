//! Build-time contract generation for roundhouse.
//!
//! This is the library a `build.rs` uses:
//!
//! ```text
//! roundhouse_build::prepare()             // write .roundhouse/contract.json + openapi.json
//! roundhouse_build::prepare_to(path)      // the same, into another directory
//! ```
//!
//! Inspection commands (`check`, `explain`, `verify`, `diff`) belong to developers and CI, and
//! live in the `roundhouse-cli` binary.
//!
//! Missing artifacts are seeded with empty placeholders before the harness runs. The harness
//! compiles the application as a dependency first, so `include_contract!` must be readable even
//! on a clean checkout; successful generation replaces the placeholders.
//!
//! A `build.rs` should call [`prepare`] so the artifacts are refreshed before rustc reads
//! `include_contract!`:
//!
//! ```ignore
//! fn main() {
//!     println!("cargo:rerun-if-changed=src");
//!     if std::env::var_os("ROUNDHOUSE_BUILDING_CONTRACT").is_some() {
//!         return;
//!     }
//!     roundhouse_build::prepare().expect("roundhouse contract generation failed");
//! }
//! ```
//!
//! Add it as a build-dependency. `cargo roundhouse init`, in the separate `roundhouse-cli`
//! binary, writes the dependency and the script:
//!
//! ```toml
//! [build-dependencies]
//! roundhouse-build = { version = "0.4" }
//! ```
//!
//! The generated harness lives under `target/roundhouse`; nothing outside `target/` and
//! `.roundhouse/` is written. Generation is one strict build: a handler with no documented
//! success fails `prepare` instead of being guessed. The harness run sets
//! `ROUNDHOUSE_BUILDING_CONTRACT`, so an application build script does not recurse.
//!
//! Before the harness runs, [`analyze`] parses the library and writes
//! `target/roundhouse/handlers.json`. `#[handler]` reads that index to document opaque
//! `impl IntoResponse` returns; the generated typed calls are verified by the compiler. [`prepare`]
//! also exports the index path with `cargo:rustc-env`, so the crate's own compile sees the same
//! data as the harness. Named return types do not use the index at all.

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

pub mod analyze;

/// The directory `prepare` writes to by default.
pub const ARTIFACT_DIR: &str = ".roundhouse";

/// A runtime-shaped contract used while the application compiles for the first time.
const PLACEHOLDER_CONTRACT: &str = r#"{
  "kind": "roundhouse.contract",
  "contract_version": 1,
  "generator_version": "pending",
  "openapi": "openapi.json",
  "operations": [],
  "validation": {
    "defs": {},
    "bodies": { "ops": {}, "types": {} },
    "parts": { "types": {} }
  },
  "diagnostics": []
}"#;

/// A minimal OpenAPI document used with [`PLACEHOLDER_CONTRACT`].
const PLACEHOLDER_OPENAPI: &str = r#"{
  "openapi": "3.1.0",
  "info": { "title": "pending", "version": "0" },
  "paths": {}
}"#;

/// Generate the contract into `.roundhouse` in the current directory.
///
/// This is the `build.rs` entry point. It never edits `src/` or `Cargo.toml`.
pub fn prepare() -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    prepare_to(PathBuf::from(ARTIFACT_DIR))?;
    // Make the index available to the crate's own compile as well. The generated documentation
    // code only exists under `build`, but the env is harmless in a production build and keeps
    // the macro's lookup consistent between the harness and the normal crate compile.
    let index = cwd
        .join("target")
        .join("roundhouse")
        .join(analyze::INDEX_FILE);
    println!(
        "cargo:rustc-env=ROUNDHOUSE_HANDLER_INDEX={}",
        index.display()
    );
    Ok(())
}

/// Generate the contract into `out`, relative to the current directory.
///
/// A `build.rs` can call this directly; Cargo runs build scripts with the package root as the
/// working directory.
///
/// Before the harness runs, missing artifacts are seeded with empty placeholders. The harness
/// compiles the application as a dependency first, so `include_contract!` must be able to read
/// its files even on a clean checkout. The real artifacts replace the placeholders once the
/// contract build succeeds.
pub fn prepare_to(out: PathBuf) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(&out)?;
    ensure_artifacts(&out)?;
    // The application normally reads `.roundhouse/` relative to its manifest. Keep that readable
    // too when `prepare --out` targets another directory.
    let default = PathBuf::from(ARTIFACT_DIR);
    if out != default {
        fs::create_dir_all(&default)?;
        ensure_artifacts(&default)?;
    }
    let cwd = std::env::current_dir()?;
    let index = analyze::analyze(&cwd)?;
    let index_path = analyze::write_index(&cwd, &index)?;
    generate(&out, &index_path, &cwd)
}

/// Write empty, valid artifacts when they do not exist yet.
fn ensure_artifacts(out: &Path) -> Result<(), Box<dyn Error>> {
    let contract = out.join("contract.json");
    if !contract.exists() {
        fs::write(&contract, format!("{PLACEHOLDER_CONTRACT}\n"))?;
    }
    let openapi = out.join("openapi.json");
    if !openapi.exists() {
        fs::write(&openapi, format!("{PLACEHOLDER_OPENAPI}\n"))?;
    }
    Ok(())
}

/// Run the strict contract build once.
fn generate(out: &Path, index_path: &Path, cwd: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(cwd.join("target"))?;
    run_contract_harness(out, index_path, cwd)
}

/// Build and run the contract binary under `target/roundhouse`.
///
/// The harness depends on the application crate and on `roundhouse` with the `build` feature.
/// Feature unification makes the application's `api!` block visible for this build only. Nothing
/// outside `target/` is written.
fn run_contract_harness(out: &Path, index_path: &Path, cwd: &Path) -> Result<(), Box<dyn Error>> {
    let package = package_info()?;
    let roundhouse = roundhouse_dependency()?;
    let harness = cwd.join("target").join("roundhouse").join("harness");
    fs::create_dir_all(harness.join("src"))?;

    let roundhouse_dep = match roundhouse.path {
        Some(path) => format!(
            "{{ path = {:?}, features = [\"build\"] }}",
            path.display().to_string()
        ),
        None => format!(
            "{{ version = \"={}\", features = [\"build\"] }}",
            roundhouse.version
        ),
    };
    let manifest = format!(
        "[package]\n\
         name = \"roundhouse-contract-harness\"\n\
         version = \"0.0.0\"\n\
         edition = \"2024\"\n\
         publish = false\n\
         \n\
         [workspace]\n\
         \n\
         [dependencies]\n\
         {} = {{ path = {:?} }}\n\
         roundhouse = {}\n",
        package.name,
        package.manifest_dir.display().to_string(),
        roundhouse_dep,
    );
    fs::write(harness.join("Cargo.toml"), manifest)?;
    fs::write(
        harness.join("src/main.rs"),
        generated_bin(&package.lib_name),
    )?;

    let status = Command::new(cargo())
        .args(["run", "--quiet", "--manifest-path"])
        .arg(harness.join("Cargo.toml"))
        .arg("--")
        .arg(out)
        .current_dir(cwd)
        // Keep the harness build inside the application's target directory; `target/` is
        // disposable, and the source tree stays untouched.
        .env("CARGO_TARGET_DIR", cwd.join("target").join("roundhouse"))
        // The harness compiles the application, whose generated `build.rs` would otherwise call
        // `prepare()` again. This guard makes contract generation re-entrant-safe.
        .env("ROUNDHOUSE_BUILDING_CONTRACT", "1")
        .env("ROUNDHOUSE_AUTO_CONTRACT", "0")
        .env("ROUNDHOUSE_HANDLER_INDEX", index_path)
        .status()?;
    if !status.success() {
        return Err(error(
            "the contract harness failed; check the `api!` block and the build output",
        ));
    }
    Ok(())
}

fn roundhouse_dependency() -> Result<RoundhouseDependency, Box<dyn Error>> {
    let output = Command::new(cargo())
        .args(["metadata", "--format-version", "1"])
        .output()?;
    if !output.status.success() {
        return Err(error(format!(
            "`cargo metadata` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let package = metadata["packages"]
        .as_array()
        .and_then(|packages| {
            packages
                .iter()
                .find(|package| package["name"].as_str() == Some("roundhouse"))
        })
        .ok_or_else(|| error("could not find the `roundhouse` dependency in `cargo metadata`"))?;
    let version = package["version"]
        .as_str()
        .ok_or_else(|| error("the `roundhouse` package has no version"))?
        .to_string();
    let path = package["source"]
        .is_null()
        .then(|| {
            package["manifest_path"]
                .as_str()
                .and_then(|path| Path::new(path).parent())
                .map(Path::to_path_buf)
        })
        .flatten();
    Ok(RoundhouseDependency { path, version })
}

struct RoundhouseDependency {
    path: Option<PathBuf>,
    version: String,
}

fn package_info() -> Result<PackageInfo, Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let manifest_path = cwd.join("Cargo.toml");
    if !manifest_path.exists() {
        return Err(error("no Cargo.toml in the current directory"));
    }
    let manifest_path = manifest_path.canonicalize()?;

    let output = Command::new(cargo())
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(&cwd)
        .output()?;
    if !output.status.success() {
        return Err(error(format!(
            "`cargo metadata` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| error("`cargo metadata` returned no packages"))?;
    let package = packages
        .iter()
        .find(|package| {
            package["manifest_path"]
                .as_str()
                .and_then(|path| Path::new(path).canonicalize().ok())
                .as_ref()
                == Some(&manifest_path)
        })
        .ok_or_else(|| {
            error(
                "could not find the current package in `cargo metadata`; run the command from a \
                 package directory",
            )
        })?;

    let name = package["name"]
        .as_str()
        .ok_or_else(|| error("package name is missing from `cargo metadata`"))?
        .to_string();
    let lib_name = package["targets"]
        .as_array()
        .and_then(|targets| {
            targets.iter().find_map(|target| {
                let is_lib = target["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("lib")));
                is_lib.then(|| target["name"].as_str()).flatten()
            })
        })
        .ok_or_else(|| {
            error(
                "contract generation requires a library target; add src/lib.rs or move the route \
                 modules into a lib and keep main.rs as a thin binary",
            )
        })?
        .to_string();

    Ok(PackageInfo {
        name,
        lib_name,
        manifest_dir: manifest_path
            .parent()
            .ok_or_else(|| error("the package manifest has no parent directory"))?
            .to_path_buf(),
    })
}

fn generated_bin(lib_name: &str) -> String {
    format!(
        "// @generated by roundhouse. Do not edit.\n\
         // Built under target/roundhouse by `cargo roundhouse prepare`.\n\
         \n\
         #[rustfmt::skip]\n\
         fn main() -> Result<(), Box<dyn std::error::Error>> {{\n\
             let output = std::env::args()\n\
                 .nth(1)\n\
                 .unwrap_or_else(|| \"{ARTIFACT_DIR}\".to_string());\n\
             let contract = {lib_name}::__roundhouse_contract();\n\
             let built = contract.build()?;\n\
             built.write_to(output)?;\n\
             Ok(())\n\
         }}\n"
    )
}

fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

fn error(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}

struct PackageInfo {
    name: String,
    lib_name: String,
    manifest_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_bin_calls_the_contract_function_and_accepts_an_output() {
        let bin = generated_bin("notes");
        assert!(bin.starts_with("// @generated by roundhouse"));
        assert!(bin.contains("notes::__roundhouse_contract()"));
        assert!(bin.contains(".roundhouse"));
        assert!(!bin.contains("inference"));
        assert!(!bin.contains("discovery"));
        assert!(bin.contains("contract.build()?"));
        assert!(bin.contains("built.write_to(output)?"));
    }

    #[test]
    fn placeholders_are_written_only_when_the_artifacts_are_missing() {
        let dir = std::env::temp_dir().join(format!(
            "roundhouse-placeholder-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temporary directory");

        ensure_artifacts(&dir).expect("seed placeholders");
        let contract: Value = serde_json::from_str(
            &fs::read_to_string(dir.join("contract.json")).expect("read contract"),
        )
        .expect("placeholder contract is JSON");
        assert_eq!(contract["kind"], "roundhouse.contract");
        assert!(contract["operations"].is_array());
        serde_json::from_str::<Value>(
            &fs::read_to_string(dir.join("openapi.json")).expect("read openapi"),
        )
        .expect("placeholder openapi is JSON");

        fs::write(dir.join("contract.json"), "{\"real\":true}").expect("write real contract");
        ensure_artifacts(&dir).expect("keep the real contract");
        assert_eq!(
            fs::read_to_string(dir.join("contract.json")).expect("read contract"),
            "{\"real\":true}"
        );

        fs::remove_dir_all(&dir).expect("clean temporary directory");
    }
}
