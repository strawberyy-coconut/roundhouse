use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut check = false;
    let mut paths = Vec::new();
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--check" => check = true,
            "-h" | "--help" => {
                println!(
                    "usage: roundhouse-codemod [--check] <file-or-directory>...\n\n\
                     Rewrites #[utoipa::path(..)] handlers to #[handler] and emits routes! tables.\n\
                     With --check, exits 1 when a file would change."
                );
                return ExitCode::SUCCESS;
            }
            other => paths.push(PathBuf::from(other)),
        }
    }
    if paths.is_empty() {
        eprintln!("roundhouse-codemod: no files given (try --help)");
        return ExitCode::from(2);
    }

    let mut files = Vec::new();
    for path in &paths {
        collect(path, &mut files);
    }

    let mut changed = false;
    let mut failed = false;
    for path in files {
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("{}: {error}", path.display());
                failed = true;
                continue;
            }
        };
        if !roundhouse_codemod::looks_unmigrated(&source) {
            continue;
        }
        let migration = match roundhouse_codemod::migrate(&source) {
            Ok(migration) => migration,
            Err(error) => {
                eprintln!("{}: {error}", path.display());
                failed = true;
                continue;
            }
        };
        for warning in &migration.warnings {
            eprintln!("{}: warning: {warning}", path.display());
        }
        if migration.source == source {
            continue;
        }
        changed = true;
        if check {
            println!("would rewrite {}", path.display());
        } else {
            if let Err(error) = std::fs::write(&path, &migration.source) {
                eprintln!("{}: {error}", path.display());
                failed = true;
                continue;
            }
            println!("rewrote {}", path.display());
        }
    }

    if failed {
        ExitCode::FAILURE
    } else if check && changed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path.to_path_buf());
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        files.push(path.to_path_buf());
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}
