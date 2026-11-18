//! `cargo roundhouse init` / `cargo roundhouse new`: scaffold a package, create `.roundhouse/`,
//! add the build-dependency and write `build.rs`.
//!
//! `init` follows Cargo's conventions: it initializes the package in an existing directory,
//! creating one with `cargo init --lib` when there is no manifest. `new` creates the directory
//! first, like `cargo new`.

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use roundhouse_build::ARTIFACT_DIR;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue, value};

use crate::error;

pub fn init(
    path: Option<PathBuf>,
    name: Option<String>,
    no_build_script: bool,
) -> Result<(), Box<dyn Error>> {
    let directory = match path {
        Some(path) => absolute(path)?,
        None => std::env::current_dir()?,
    };
    fs::create_dir_all(&directory)?;
    if !directory.is_dir() {
        return Err(error(format!("{} is not a directory", directory.display())));
    }

    let fresh = !directory.join("Cargo.toml").exists();
    let sources = Sources::capture(&directory);
    if fresh {
        cargo_init(&directory, name.as_deref())?;
    }

    finish(&directory, fresh, sources, no_build_script)
}

pub fn new(
    path: PathBuf,
    name: Option<String>,
    no_build_script: bool,
) -> Result<(), Box<dyn Error>> {
    let directory = absolute(path)?;
    if directory.exists() {
        return Err(error(format!(
            "destination `{}` already exists",
            directory.display()
        )));
    }

    cargo_new(&directory, name.as_deref())?;
    finish(&directory, true, Sources::default(), no_build_script)
}

/// Which source files existed before `init` created the package.
#[derive(Clone, Copy, Default)]
struct Sources {
    lib: bool,
    main: bool,
}

impl Sources {
    fn capture(directory: &Path) -> Self {
        Self {
            lib: directory.join("src/lib.rs").exists(),
            main: directory.join("src/main.rs").exists(),
        }
    }
}

fn finish(
    directory: &Path,
    fresh: bool,
    sources: Sources,
    no_build_script: bool,
) -> Result<(), Box<dyn Error>> {
    let name = package_name(directory)?;
    if !fresh && !has_library_target(directory) {
        return Err(error(
            "contract generation requires a library target; add src/lib.rs or move the route \
             modules into a lib and keep main.rs as a thin binary, then run `cargo roundhouse \
             init` again",
        ));
    }

    fs::create_dir_all(directory.join(ARTIFACT_DIR))?;
    // `init --ephemeral` used to leave a marker; the harness is now the only mode.
    let marker = directory.join(ARTIFACT_DIR).join("ephemeral");
    if marker.exists() {
        fs::remove_file(marker)?;
    }

    if fresh {
        install_dependencies(directory)?;
        write_sources(directory, &name, sources)?;
    }

    if !no_build_script {
        install_build_script(directory)?;
    }

    println!("created {ARTIFACT_DIR}/; the contract binary is generated under target/roundhouse");
    let old_bin = directory.join("src/bin/roundhouse-contract.rs");
    if old_bin.exists() {
        println!(
            "note: {} is no longer used; remove it and any `[[bin]]`/feature entry that names it",
            old_bin.display()
        );
    }

    if fresh {
        if !sources.lib {
            println!("created src/lib.rs");
        }
        if !sources.main {
            println!("created src/main.rs");
        }
        if no_build_script {
            println!(
                "note: --no-build-script was passed; run `cargo roundhouse prepare` before \
                 `cargo build` so `.roundhouse/` exists"
            );
        }
    } else {
        println!(
            "\nadd this to src/lib.rs if it is not already there:\n\n{}\nThen run `cargo build` (or `cargo roundhouse prepare`).",
            api_snippet(&name)
        );
    }
    Ok(())
}

fn cargo_init(directory: &Path, name: Option<&str>) -> Result<(), Box<dyn Error>> {
    let mut command = cargo_command();
    command.arg("init").arg("--lib");
    if let Some(name) = name {
        command.arg("--name").arg(name);
    }
    command.current_dir(directory);
    run_cargo(command, "cargo init")
}

fn cargo_new(directory: &Path, name: Option<&str>) -> Result<(), Box<dyn Error>> {
    let mut command = cargo_command();
    command.arg("new").arg("--lib");
    if let Some(name) = name {
        command.arg("--name").arg(name);
    }
    command.arg(directory);
    run_cargo(command, "cargo new")
}

fn cargo_command() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
}

fn run_cargo(mut command: Command, what: &str) -> Result<(), Box<dyn Error>> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(error(format!("`{what}` failed with {status}")))
    }
}

fn absolute(path: PathBuf) -> Result<PathBuf, Box<dyn Error>> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn has_library_target(directory: &Path) -> bool {
    if directory.join("src/lib.rs").is_file() {
        return true;
    }
    fs::read_to_string(directory.join("Cargo.toml"))
        .ok()
        .and_then(|manifest| manifest.parse::<DocumentMut>().ok())
        .is_some_and(|manifest| manifest.get("lib").is_some())
}

fn package_name(directory: &Path) -> Result<String, Box<dyn Error>> {
    let manifest = fs::read_to_string(directory.join("Cargo.toml"))?;
    let manifest: DocumentMut = manifest.parse()?;
    manifest
        .get("package")
        .and_then(Item::as_table)
        .and_then(|package| package.get("name"))
        .and_then(Item::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            error(
                "no `[package]` in Cargo.toml; run this inside a package directory, not a \
                 workspace root",
            )
        })
}

/// Add the runtime dependencies and the test-only contract feature to a freshly created package.
fn install_dependencies(directory: &Path) -> Result<(), Box<dyn Error>> {
    let manifest_path = directory.join("Cargo.toml");
    let original = fs::read_to_string(&manifest_path)?;
    let mut manifest: DocumentMut = original.parse()?;
    let version = minor_version(env!("CARGO_PKG_VERSION"));

    let dependencies = table(&mut manifest, "dependencies")?;
    insert_dependency_without_defaults(
        dependencies,
        "roundhouse",
        &version,
        &["contract", "server", "swagger-ui"],
    );
    insert_dependency(dependencies, "serde", "1", &["derive"]);
    insert_dependency(dependencies, "tokio", "1", &["macros", "rt-multi-thread"]);

    let dev_dependencies = table(&mut manifest, "dev-dependencies")?;
    insert_dependency_without_defaults(dev_dependencies, "roundhouse", &version, &["build"]);

    let updated = manifest.to_string();
    if updated != original {
        fs::write(&manifest_path, updated)?;
        println!("updated Cargo.toml with the roundhouse dependencies");
    }
    Ok(())
}

fn table<'a>(manifest: &'a mut DocumentMut, key: &str) -> Result<&'a mut Table, Box<dyn Error>> {
    manifest
        .entry(key)
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| error(format!("`[{key}]` in Cargo.toml is not a table")))
}

fn insert_dependency_without_defaults(
    table: &mut Table,
    name: &str,
    version: &str,
    features: &[&str],
) {
    insert_dependency(table, name, version, features);
    if let Some(Item::Value(TomlValue::InlineTable(spec))) = table.get_mut(name) {
        spec.insert("default-features", TomlValue::from(false));
    }
}

fn insert_dependency(table: &mut Table, name: &str, version: &str, features: &[&str]) {
    if table.contains_key(name) {
        return;
    }
    let mut spec = InlineTable::new();
    spec.insert("version", TomlValue::from(version));
    if !features.is_empty() {
        let mut array = Array::new();
        for feature in features {
            array.push(*feature);
        }
        spec.insert("features", TomlValue::Array(array));
    }
    table.insert(name, value(spec));
}

fn write_sources(directory: &Path, package: &str, sources: Sources) -> Result<(), Box<dyn Error>> {
    let lib_path = directory.join("src/lib.rs");
    if !sources.lib {
        fs::create_dir_all(directory.join("src"))?;
        fs::write(&lib_path, LIBRARY_TEMPLATE)?;
    }

    let main_path = directory.join("src/main.rs");
    if !sources.main {
        fs::write(&main_path, main_template(package))?;
    }
    Ok(())
}

/// Add the `roundhouse-build` build-dependency and generate a guarded `build.rs`.
///
/// An existing hand-written `build.rs` is never overwritten; the roundhouse block is printed
/// instead.
fn install_build_script(directory: &Path) -> Result<(), Box<dyn Error>> {
    let manifest_path = directory.join("Cargo.toml");
    let original = fs::read_to_string(&manifest_path)?;
    let mut manifest: DocumentMut = original.parse()?;

    let dependency = build_dependency_spec(&manifest, directory);
    let build_dependencies = manifest
        .entry("build-dependencies")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| error("`[build-dependencies]` in Cargo.toml is not a table"))?;
    // `roundhouse-cli` was the 0.4.0 build-dependency; the generator now lives in
    // `roundhouse-build` and the CLI is only the binary.
    let migrated = build_dependencies.remove("roundhouse-cli").is_some();
    if !build_dependencies.contains_key("roundhouse-build") {
        build_dependencies.insert("roundhouse-build", value(dependency));
    }
    let updated = manifest.to_string();
    if updated != original {
        if migrated {
            println!("replaced the roundhouse-cli build-dependency with roundhouse-build");
        }
        fs::write(&manifest_path, updated)?;
        println!("updated Cargo.toml with the roundhouse-build build-dependency");
    }

    let build_path = directory.join("build.rs");
    let expected = generated_build_script();
    match build_path
        .exists()
        .then(|| fs::read_to_string(&build_path))
        .transpose()?
    {
        Some(existing) if existing == expected => {}
        Some(existing) if existing.starts_with("// @generated by roundhouse") => {
            fs::write(&build_path, expected)?;
            println!("updated build.rs");
        }
        Some(_) => {
            println!(
                "note: build.rs exists and was not written by roundhouse; add this block to it:\n\n{}",
                build_script_body()
            );
        }
        None => {
            fs::write(&build_path, expected)?;
            println!("created build.rs");
        }
    }
    Ok(())
}

fn build_dependency_spec(manifest: &DocumentMut, directory: &Path) -> InlineTable {
    let roundhouse = manifest
        .get("dependencies")
        .and_then(Item::as_table_like)
        .and_then(|dependencies| dependencies.get("roundhouse"));

    let mut spec = InlineTable::new();

    let (path, version) = match roundhouse {
        Some(Item::Value(TomlValue::InlineTable(table))) => (
            table.get("path").and_then(TomlValue::as_str),
            table.get("version").and_then(TomlValue::as_str),
        ),
        Some(Item::Value(TomlValue::String(version))) => (None, Some(version.value().as_str())),
        Some(Item::Table(table)) => (
            table.get("path").and_then(Item::as_str),
            table.get("version").and_then(Item::as_str),
        ),
        _ => (None, None),
    };

    if let Some(path) = path {
        let build = Path::new(path).join("roundhouse-build");
        if directory.join(&build).join("Cargo.toml").exists() {
            spec.insert(
                "path",
                TomlValue::from(build.to_string_lossy().replace('\\', "/")),
            );
            return spec;
        }
    }

    let version = version
        .map(minor_version)
        .unwrap_or_else(|| minor_version(env!("CARGO_PKG_VERSION")));
    spec.insert("version", TomlValue::from(version));
    spec
}

fn minor_version(version: &str) -> String {
    let mut parts = version.split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) => format!("{major}.{minor}"),
        _ => version.to_string(),
    }
}

fn generated_build_script() -> String {
    let body = build_script_body()
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "// @generated by roundhouse. Do not edit.\n\
         // Regenerate with `cargo roundhouse init`.\n\
         \n\
         fn main() {{\n{body}\n}}\n"
    )
}

fn build_script_body() -> &'static str {
    r#"println!("cargo:rerun-if-changed=src");
println!("cargo:rerun-if-changed=Cargo.toml");
println!("cargo:rerun-if-changed=Cargo.lock");
println!("cargo:rerun-if-env-changed=ROUNDHOUSE_AUTO_CONTRACT");
println!("cargo:rerun-if-env-changed=CI");
if std::env::var_os("ROUNDHOUSE_BUILDING_CONTRACT").is_some() {
    return;
}
if std::env::var_os("ROUNDHOUSE_AUTO_CONTRACT").is_some_and(|value| value == "0") {
    return;
}
// CI builds the committed artifact; `cargo roundhouse check` verifies it. Set
// ROUNDHOUSE_AUTO_CONTRACT=1 to opt back in.
if std::env::var_os("CI").is_some()
    && std::env::var_os("ROUNDHOUSE_AUTO_CONTRACT").is_none()
{
    return;
}
if let Err(error) = roundhouse_build::prepare() {
    panic!("roundhouse contract generation failed: {error}");
}"#
}

fn api_snippet(name: &str) -> String {
    format!(
        "roundhouse::api! {{\n\
         \x20   state = crate::__RoundhouseAppState,\n\
         \x20   title = {name:?},\n\
         \x20   version = env!(\"CARGO_PKG_VERSION\"),\n\
         \x20   root = routes::MODULE,\n\
         }}"
    )
}

fn main_template(package: &str) -> String {
    MAIN_TEMPLATE.replace("__CRATE__", &package.replace('-', "_"))
}

const LIBRARY_TEMPLATE: &str = r#"//! The application: state, schemas, routes and handlers.
//!
//! Keep the declarations in this library and `main.rs` as a thin server, so `cargo roundhouse
//! prepare` can read the same module tree that the runtime builds.

use roundhouse::prelude::*;

roundhouse::app_state!(AppState);

roundhouse::api! {
    state = AppState,
    title = env!("CARGO_PKG_NAME"),
    version = env!("CARGO_PKG_VERSION"),
    root = routes::MODULE,
}

/// Application state shared by the handlers.
#[derive(Clone, Default)]
pub struct AppState;

#[roundhouse::schema]
#[derive(serde::Serialize)]
pub struct Hello {
    /// The greeting.
    pub message: String,
}

mod routes {
    use super::*;

    routes! {
        "/hello" => {
            get(hello),
        },
    }

    /// @summary: Say hello
    #[handler]
    pub async fn hello() -> Json<Hello> {
        Json(Hello {
            message: "Hello from roundhouse".to_string(),
        })
    }
}

/// Build the application.
pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state, routes)
        .contract(roundhouse::include_contract!())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_builds() {
        app(AppState::default())
            .try_build()
            .expect("the app builds");
    }
}
"#;

const MAIN_TEMPLATE: &str = r#"//! Serve the application.

use __CRATE__::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    __CRATE__::app(AppState::default())
        .serve("0.0.0.0:3000")
        .await?;
    Ok(())
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_build_script_guards_recursion_and_ci() {
        let script = generated_build_script();
        assert!(script.starts_with("// @generated by roundhouse"));
        assert!(script.contains("ROUNDHOUSE_BUILDING_CONTRACT"));
        assert!(script.contains("ROUNDHOUSE_AUTO_CONTRACT"));
        assert!(script.contains("roundhouse_build::prepare()"));
    }

    #[test]
    fn init_replaces_the_cli_build_dependency() {
        let dir = std::env::temp_dir().join(format!("roundhouse-init-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temporary package");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\n\
             name = \"example\"\n\
             version = \"0.1.0\"\n\
             edition = \"2024\"\n\
             \n\
             [dependencies]\n\
             roundhouse = \"0.4\"\n\
             \n\
             [build-dependencies]\n\
             roundhouse-cli = { version = \"0.4\", default-features = false }\n",
        )
        .expect("write manifest");

        install_build_script(&dir).expect("install the generated build script");

        let manifest = fs::read_to_string(dir.join("Cargo.toml")).expect("read manifest");
        assert!(manifest.contains("roundhouse-build"));
        assert!(!manifest.contains("roundhouse-cli"));
        assert!(!manifest.contains("default-features"));
        let script = fs::read_to_string(dir.join("build.rs")).expect("read build script");
        assert!(script.contains("roundhouse_build::prepare()"));

        fs::remove_dir_all(&dir).expect("remove temporary package");
    }

    #[test]
    fn api_snippet_uses_the_module_constant() {
        let snippet = api_snippet("Notes API");
        assert!(snippet.contains("title = \"Notes API\""));
        assert!(snippet.contains("root = routes::MODULE"));
        assert!(snippet.contains("state = crate::__RoundhouseAppState"));
    }
}
