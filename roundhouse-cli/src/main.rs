//! `cargo roundhouse` — build-time contract generation.
//!
//! The CLI owns the generated `target/roundhouse` harness and the `.roundhouse/` artifacts.
//! `prepare` runs the generated contract builder plus source inference and writes both artifacts,
//! `check` verifies they are current, `explain` shows one operation's responses, and `verify`
//! runs tests with contract response checking enabled.

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Parser, Subcommand};
use serde_json::Value;

mod infer;
mod semantic;

const ARTIFACT_DIR: &str = ".roundhouse";

#[derive(Parser)]
#[command(
    name = "cargo-roundhouse",
    bin_name = "cargo roundhouse",
    version,
    about = "Build-time contract generation for roundhouse",
    long_about = "Build-time contract generation for roundhouse.\n\n\
                  The contract binary is built under `target/roundhouse`, so `cargo roundhouse` \
                  never writes a bin, edits `Cargo.toml`, or otherwise touches the source tree."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create .roundhouse/ and print the api! snippet
    Init,
    /// Run the contract harness and write contract.json + openapi.json
    Prepare {
        /// Directory that receives contract.json and openapi.json
        #[arg(long, default_value = ARTIFACT_DIR)]
        out: PathBuf,
    },
    /// Verify the committed contract artifacts are current
    Check,
    /// Show the documented responses for one operation
    Explain {
        /// Operation id, path, or `METHOD /path`
        operation: String,
    },
    /// Run tests with response verification against the committed contract
    Verify {
        /// Arguments passed to `cargo test` after `--`
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Compare the committed contract with a previous version
    Diff {
        /// Compare against this contract.json instead of `git HEAD`
        #[arg(long)]
        against: Option<PathBuf>,
        /// Exit non-zero when a breaking change is found
        #[arg(long)]
        check: bool,
    },
}

fn main() {
    let cli = Cli::parse_from(normalized_args());
    if let Err(error) = run(cli) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        Commands::Init => init(),
        Commands::Prepare { out } => prepare(out),
        Commands::Check => check(),
        Commands::Explain { operation } => explain(&operation),
        Commands::Verify { args } => verify(args),
        Commands::Diff { against, check } => diff(against, check),
    }
}

/// Cargo may invoke the subcommand as `cargo-roundhouse roundhouse …` depending on the
/// installation; accept both `cargo roundhouse init` and `cargo-roundhouse roundhouse init`.
fn normalized_args() -> Vec<OsString> {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).and_then(|arg| arg.to_str()) == Some("roundhouse") {
        args.remove(1);
    }
    args
}

fn init() -> Result<(), Box<dyn Error>> {
    let package = package_info()?;
    let cwd = std::env::current_dir()?;
    fs::create_dir_all(cwd.join(ARTIFACT_DIR))?;
    // `init --ephemeral` used to leave a marker; the harness is now the only mode.
    let marker = cwd.join(ARTIFACT_DIR).join("ephemeral");
    if marker.exists() {
        fs::remove_file(marker)?;
    }
    println!(
        "created {ARTIFACT_DIR}/; the contract binary is generated under target/roundhouse, \
         so src/ and Cargo.toml stay untouched"
    );
    let old_bin = cwd.join("src/bin/roundhouse-contract.rs");
    if old_bin.exists() {
        println!(
            "note: {} is no longer used; remove it and any `[[bin]]`/feature entry that names it",
            old_bin.display()
        );
    }
    println!(
        "\nadd this to src/lib.rs if it is not already there:\n\n{}\nThen run `cargo roundhouse prepare`.",
        api_snippet(&package.name)
    );
    Ok(())
}

fn prepare(out: PathBuf) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(&out)?;
    generate(&out)?;
    summarize(&out)?;
    Ok(())
}

/// Run the discovery pass, infer responses, then run the strict contract build.
fn generate(out: &Path) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let target = cwd.join("target");
    fs::create_dir_all(&target)?;
    let discovery = target.join(format!("roundhouse-discovery-{}", std::process::id()));
    let inference = target.join(format!("roundhouse-inference-{}.json", std::process::id()));
    if discovery.exists() {
        fs::remove_dir_all(&discovery)?;
    }
    fs::create_dir_all(&discovery)?;

    let result = (|| -> Result<(), Box<dyn Error>> {
        run_contract_bin(&discovery, &["--discovery"])?;
        infer::analyze(&discovery, &inference)?;
        run_contract_bin(out, &["--inference", &inference.to_string_lossy()])?;
        Ok(())
    })();

    let _ = fs::remove_dir_all(&discovery);
    let _ = fs::remove_file(&inference);
    result
}

fn check() -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let committed = cwd.join(ARTIFACT_DIR);
    if !committed.join("contract.json").exists() || !committed.join("openapi.json").exists() {
        return Err(error(
            "no committed contract found; run `cargo roundhouse prepare` and commit .roundhouse/",
        ));
    }

    let temporary = cwd
        .join("target")
        .join(format!("roundhouse-check-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_dir_all(&temporary)?;
    }
    fs::create_dir_all(&temporary)?;

    let result = generate(&temporary).and_then(|()| compare_artifacts(&committed, &temporary));
    let _ = fs::remove_dir_all(&temporary);
    result?;

    println!("roundhouse contract is current");
    Ok(())
}

fn verify(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    if !cwd.join(ARTIFACT_DIR).join("contract.json").exists() {
        return Err(error(
            "no committed contract found; run `cargo roundhouse prepare` first",
        ));
    }

    let mut command = Command::new(cargo());
    command.arg("test");
    if !args.is_empty() {
        command.arg("--");
        command.args(args);
    }
    command.env("ROUNDHOUSE_VERIFY", "1");

    let status = command.status()?;
    if !status.success() {
        return Err(error(
            "response verification failed; a response did not match .roundhouse/contract.json",
        ));
    }
    Ok(())
}

fn explain(operation: &str) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let path = cwd.join(ARTIFACT_DIR).join("contract.json");
    let contract: Value = serde_json::from_str(&fs::read_to_string(&path).map_err(|_| {
        error("no committed contract found; run `cargo roundhouse prepare` first")
    })?)?;
    let operations = contract["operations"]
        .as_array()
        .ok_or_else(|| error("contract has no operations"))?;

    let wanted = operation.trim();
    let wanted_method_path = wanted.split_once(' ');
    let found = operations.iter().find(|candidate| {
        let id = candidate["operation_id"].as_str().unwrap_or_default();
        let path = candidate["path"].as_str().unwrap_or_default();
        let method = candidate["method"].as_str().unwrap_or_default();
        id == wanted
            || path == wanted
            || wanted_method_path.is_some_and(|(wanted_method, wanted_path)| {
                method.eq_ignore_ascii_case(wanted_method) && path == wanted_path
            })
    });

    let Some(operation) = found else {
        let ids: Vec<&str> = operations
            .iter()
            .filter_map(|operation| operation["operation_id"].as_str())
            .collect();
        return Err(error(format!(
            "no operation matched `{wanted}`; known operations: {}",
            ids.join(", ")
        )));
    };

    println!(
        "{} {} ({})",
        operation["method"].as_str().unwrap_or("?"),
        operation["path"].as_str().unwrap_or("?"),
        operation["operation_id"].as_str().unwrap_or("?")
    );
    if let (Some(file), Some(line)) = (
        operation["source"]["file"].as_str(),
        operation["source"]["line"].as_u64(),
    ) {
        println!("  at {file}:{line}");
    }
    let Some(responses) = operation["responses"].as_array() else {
        println!("  no responses documented");
        return Ok(());
    };
    for response in responses {
        let status = response["status"].as_u64().map_or_else(
            || "default".to_string(),
            |status| {
                if status == 0 {
                    "default".to_string()
                } else {
                    status.to_string()
                }
            },
        );
        let content_type = response["content"]
            .as_object()
            .and_then(|content| content.keys().next())
            .map_or("-", String::as_str);
        let schema_value = &response["content"][content_type]["schema"];
        let schema = schema_value["$ref"]
            .as_str()
            .or_else(|| schema_value.is_object().then_some("(inline)"))
            .unwrap_or("-");
        let provenance = response["provenance"].as_str().unwrap_or("declared");
        let description = response["description"].as_str().unwrap_or("");
        println!("  {status:<7} {content_type:<28} {schema:<36} {provenance:<9} {description}");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Breaking,
    Compatible,
}

#[derive(Debug, Clone)]
struct Change {
    severity: Severity,
    route: Option<(String, String)>,
    detail: String,
}

fn diff(against: Option<PathBuf>, check: bool) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let current_path = cwd.join(ARTIFACT_DIR).join("contract.json");
    if !current_path.exists() {
        return Err(error(
            "no committed contract found; run `cargo roundhouse prepare` first",
        ));
    }
    let current: Value = serde_json::from_str(&fs::read_to_string(&current_path)?)?;

    let previous: Value = if let Some(path) = against {
        serde_json::from_str(
            &fs::read_to_string(path)
                .map_err(|_| error("could not read the `--against` contract"))?,
        )?
    } else {
        serde_json::from_str(&git_show_contract(&cwd, &current_path)?)?
    };

    let changes = diff_contracts(&previous, &current);
    let breaking = changes
        .iter()
        .filter(|change| change.severity == Severity::Breaking)
        .count();
    let compatible = changes.len() - breaking;

    let mut grouped: std::collections::BTreeMap<Option<(String, String)>, Vec<&Change>> =
        std::collections::BTreeMap::new();
    for change in &changes {
        grouped
            .entry(change.route.clone())
            .or_default()
            .push(change);
    }
    for (route, route_changes) in grouped {
        if let Some((method, path)) = route {
            println!("{method} {path}");
            for change in route_changes {
                println!("  {:<10} {}", label(change.severity), change.detail);
            }
        } else {
            for change in route_changes {
                println!("{:<10} {}", label(change.severity), change.detail);
            }
        }
    }

    println!("{breaking} breaking, {compatible} compatible");
    if check && breaking > 0 {
        return Err(error(format!(
            "{breaking} breaking contract change(s); review them or omit `--check`"
        )));
    }
    Ok(())
}

fn label(severity: Severity) -> &'static str {
    match severity {
        Severity::Breaking => "breaking",
        Severity::Compatible => "compatible",
    }
}

fn git_show_contract(cwd: &Path, contract_path: &Path) -> Result<String, Box<dyn Error>> {
    let root = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()?;
    if !root.status.success() {
        return Err(error(
            "not in a git repository; pass `--against <contract.json>`",
        ));
    }
    let root = String::from_utf8(root.stdout)?;
    let root = Path::new(root.trim());
    let path = contract_path
        .canonicalize()?
        .strip_prefix(root)
        .map_err(|_| error("the contract is outside the git repository"))?
        .to_string_lossy()
        .replace('\\', "/");
    let output = Command::new("git")
        .args(["show", &format!("HEAD:{path}")])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(error(
            "no contract in git HEAD; commit one or pass `--against <contract.json>`",
        ));
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn diff_contracts(previous: &Value, current: &Value) -> Vec<Change> {
    use std::collections::{BTreeMap, BTreeSet};

    type Route = (String, String);
    fn routes(contract: &Value) -> BTreeMap<Route, &Value> {
        contract["operations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|operation| {
                Some((
                    (
                        operation["method"].as_str()?.to_string(),
                        operation["path"].as_str()?.to_string(),
                    ),
                    operation,
                ))
            })
            .collect()
    }
    fn responses(operation: &Value) -> BTreeMap<String, BTreeSet<String>> {
        operation["responses"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|response| {
                let status = response["status"].as_u64()?;
                let content = response["content"]
                    .as_object()
                    .map(|content| content.keys().cloned().collect())
                    .unwrap_or_default();
                Some((status.to_string(), content))
            })
            .collect()
    }
    fn request_body(operation: &Value) -> Option<(String, String)> {
        let body = &operation["request"]["body"];
        if body.is_null() {
            return None;
        }
        let content_type = body["content_type"].as_str()?.to_string();
        let schema = body["schema"]["$ref"]
            .as_str()
            .unwrap_or("<inline>")
            .to_string();
        Some((content_type, schema))
    }
    fn security(operation: &Value) -> Vec<String> {
        operation["security"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|scheme| scheme.as_str().map(str::to_string))
            .collect()
    }

    let previous_routes = routes(previous);
    let current_routes = routes(current);
    let mut changes = Vec::new();

    for (route, previous_operation) in &previous_routes {
        let Some(current_operation) = current_routes.get(route) else {
            changes.push(Change {
                severity: Severity::Breaking,
                route: Some(route.clone()),
                detail: "operation removed".to_string(),
            });
            continue;
        };

        if previous_operation["operation_id"] != current_operation["operation_id"] {
            changes.push(Change {
                severity: Severity::Breaking,
                route: Some(route.clone()),
                detail: "operation id changed".to_string(),
            });
        }

        match (
            request_body(previous_operation),
            request_body(current_operation),
        ) {
            (None, Some(_)) => changes.push(Change {
                severity: Severity::Breaking,
                route: Some(route.clone()),
                detail: "request body added".to_string(),
            }),
            (Some(_), None) => changes.push(Change {
                severity: Severity::Compatible,
                route: Some(route.clone()),
                detail: "request body removed".to_string(),
            }),
            (Some(previous), Some(current)) if previous != current => changes.push(Change {
                severity: Severity::Breaking,
                route: Some(route.clone()),
                detail: "request body changed".to_string(),
            }),
            _ => {}
        }

        if security(previous_operation) != security(current_operation) {
            changes.push(Change {
                severity: Severity::Breaking,
                route: Some(route.clone()),
                detail: "security changed".to_string(),
            });
        }

        let previous_responses = responses(previous_operation);
        let current_responses = responses(current_operation);
        for (status, previous_content) in &previous_responses {
            let Some(current_content) = current_responses.get(status) else {
                changes.push(Change {
                    severity: Severity::Breaking,
                    route: Some(route.clone()),
                    detail: format!("response {status} removed"),
                });
                continue;
            };
            for content_type in previous_content.difference(current_content) {
                changes.push(Change {
                    severity: Severity::Breaking,
                    route: Some(route.clone()),
                    detail: format!("response {status} no longer documents {content_type}"),
                });
            }
            for content_type in current_content.difference(previous_content) {
                changes.push(Change {
                    severity: Severity::Compatible,
                    route: Some(route.clone()),
                    detail: format!("response {status} adds {content_type}"),
                });
            }
        }
        for status in current_responses.keys() {
            if !previous_responses.contains_key(status) {
                changes.push(Change {
                    severity: Severity::Compatible,
                    route: Some(route.clone()),
                    detail: format!("response {status} added"),
                });
            }
        }
    }

    for route in current_routes.keys() {
        if !previous_routes.contains_key(route) {
            changes.push(Change {
                severity: Severity::Compatible,
                route: Some(route.clone()),
                detail: "operation added".to_string(),
            });
        }
    }

    changes.sort_by(|left, right| {
        left.route
            .cmp(&right.route)
            .then_with(|| left.detail.cmp(&right.detail))
    });
    changes
}

/// Build and run the contract binary under `target/roundhouse`.
///
/// The harness depends on the application crate and on `roundhouse` with the `contract` feature.
/// Feature unification makes the application's `api!` block visible for this build only. Nothing
/// outside `target/` is written.
fn run_contract_harness(out: &Path, extra: &[&str]) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let package = package_info()?;
    let roundhouse = roundhouse_dependency()?;
    let harness = cwd.join("target").join("roundhouse").join("harness");
    fs::create_dir_all(harness.join("src"))?;

    let roundhouse_dep = match roundhouse.path {
        Some(path) => format!(
            "{{ path = {:?}, features = [\"contract\"] }}",
            path.display().to_string()
        ),
        None => format!(
            "{{ version = \"={}\", features = [\"contract\"] }}",
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
        .args(extra)
        .arg(out)
        .current_dir(&cwd)
        // Keep the harness build inside the application's target directory; `target/` is
        // disposable, and the source tree stays untouched.
        .env("CARGO_TARGET_DIR", cwd.join("target").join("roundhouse"))
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

fn run_contract_bin(out: &Path, extra: &[&str]) -> Result<(), Box<dyn Error>> {
    run_contract_harness(out, extra)
}

fn summarize(out: &Path) -> Result<(), Box<dyn Error>> {
    let contract = fs::read_to_string(out.join("contract.json"))?;
    let contract: Value = serde_json::from_str(&contract)?;
    let operations = contract["operations"]
        .as_array()
        .map_or(0, |operations| operations.len());
    println!(
        "wrote {} and {} ({operations} operations)",
        out.join("contract.json").display(),
        out.join("openapi.json").display()
    );
    Ok(())
}

fn compare_artifacts(committed: &Path, generated: &Path) -> Result<(), Box<dyn Error>> {
    for name in ["contract.json", "openapi.json"] {
        let committed: Value = serde_json::from_str(&fs::read_to_string(committed.join(name))?)?;
        let generated: Value = serde_json::from_str(&fs::read_to_string(generated.join(name))?)?;
        if committed != generated {
            return Err(error(format!(
                "{name} is stale; run `cargo roundhouse prepare` and commit the result"
            )));
        }
    }
    Ok(())
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
             let mut output = \"{ARTIFACT_DIR}\".to_string();\n\
             let mut discovery = false;\n\
             let mut inference = None;\n\
             let mut args = std::env::args().skip(1);\n\
             while let Some(arg) = args.next() {{\n\
                 match arg.as_str() {{\n\
                     \"--discovery\" => discovery = true,\n\
                     \"--inference\" => inference = args.next(),\n\
                     other => output = other.to_string(),\n\
                 }}\n\
             }}\n\
             let mut contract = {lib_name}::__roundhouse_contract();\n\
             if discovery {{\n\
                 contract = contract.permissive().allow_undeclared_success(true);\n\
             }}\n\
             if let Some(path) = inference {{\n\
                 contract = contract.inference_json(&std::fs::read_to_string(path)?)?;\n\
             }}\n\
             let built = contract.build()?;\n\
             built.write_to(output)?;\n\
             Ok(())\n\
         }}\n"
    )
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
    fn diff_reports_breaking_and_compatible_changes() {
        let previous = serde_json::json!({
            "operations": [{
                "operation_id": "show",
                "method": "GET",
                "path": "/notes/{id}",
                "security": [],
                "request": { "body": null },
                "responses": [
                    { "status": 200, "content": { "application/json": {} } },
                    { "status": 404, "content": { "application/problem+json": {} } }
                ]
            }]
        });
        let current = serde_json::json!({
            "operations": [{
                "operation_id": "show",
                "method": "GET",
                "path": "/notes/{id}",
                "security": ["Bearer"],
                "request": { "body": { "content_type": "application/json", "schema": {} } },
                "responses": [
                    { "status": 200, "content": { "application/json": {} } },
                    { "status": 500, "content": { "application/problem+json": {} } }
                ]
            }]
        });

        let changes = diff_contracts(&previous, &current);
        assert!(changes.iter().any(|change| {
            change.severity == Severity::Breaking && change.detail == "security changed"
        }));
        assert!(changes.iter().any(|change| {
            change.severity == Severity::Breaking && change.detail == "request body added"
        }));
        assert!(changes.iter().any(|change| {
            change.severity == Severity::Breaking && change.detail == "response 404 removed"
        }));
        assert!(changes.iter().any(|change| {
            change.severity == Severity::Compatible && change.detail == "response 500 added"
        }));
    }

    #[test]
    fn diff_reports_removed_operations_as_breaking() {
        let previous = serde_json::json!({
            "operations": [{
                "operation_id": "old",
                "method": "GET",
                "path": "/old",
                "security": [],
                "request": { "body": null },
                "responses": []
            }]
        });
        let current = serde_json::json!({ "operations": [] });
        let changes = diff_contracts(&previous, &current);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].severity, Severity::Breaking);
        assert_eq!(changes[0].detail, "operation removed");
    }

    #[test]
    fn generated_bin_calls_the_contract_function_and_accepts_an_output() {
        let bin = generated_bin("notes");
        assert!(bin.starts_with("// @generated by roundhouse"));
        assert!(bin.contains("notes::__roundhouse_contract()"));
        assert!(bin.contains(".roundhouse"));
        assert!(bin.contains("allow_undeclared_success(true)"));
        assert!(bin.contains("inference_json"));
        assert!(bin.contains("built.write_to(output)?"));
    }

    #[test]
    fn api_snippet_uses_the_module_constant() {
        let snippet = api_snippet("Notes API");
        assert!(snippet.contains("title = \"Notes API\""));
        assert!(snippet.contains("root = routes::MODULE"));
        assert!(snippet.contains("state = crate::__RoundhouseAppState"));
    }
}
