//! `roundhouse openapi` — dump a linked app's document, for CI and golden files.
//!
//! ```sh
//! roundhouse openapi -p notes-example
//! roundhouse openapi -p notes-example --format 3.0
//! ```

use std::process::Command;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        std::process::exit(2);
    };
    if command != "openapi" && command != "routes" {
        usage();
        std::process::exit(2);
    }
    let routes = command == "routes";

    let mut package = None;
    let mut format = "3.1".to_string();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-p" | "--package" => {
                package = args.next();
            }
            "--format" => {
                if let Some(value) = args.next() {
                    format = value;
                }
            }
            "-h" | "--help" => {
                usage();
                return;
            }
            other => {
                eprintln!("unknown argument `{other}`");
                usage();
                std::process::exit(2);
            }
        }
    }

    let Some(package) = package else {
        eprintln!("missing `-p <package>`");
        usage();
        std::process::exit(2);
    };

    let flag = if routes {
        "--routes"
    } else {
        match format.as_str() {
            "3.1" | "3.1.0" => "--openapi",
            "3.0" | "3.0.3" => "--openapi-30",
            other => {
                eprintln!("unsupported format `{other}`; use 3.1 or 3.0");
                std::process::exit(2);
            }
        }
    };

    let status = Command::new("cargo")
        .args(["run", "-q", "-p", &package, "--", flag])
        .status()
        .expect("cargo is available");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn usage() {
    eprintln!(
        "usage: roundhouse <openapi|routes> -p <package> [--format 3.1|3.0]\n\
         \n\
         openapi  prints the document App::serve would have served.\n\
         routes   prints the resolved route table with each declaration's source."
    );
}
