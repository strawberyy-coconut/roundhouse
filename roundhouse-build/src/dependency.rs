//! Package metadata the slice generator needs.

use std::path::PathBuf;

/// One normal dependency declared by the application package.
#[derive(Clone, Debug)]
pub struct Dependency {
    /// The package name.
    pub name: String,
    /// The key the application uses (`rename`), when it differs from the package name.
    pub rename: Option<String>,
    pub req: String,
    pub path: Option<PathBuf>,
    pub git: Option<String>,
    pub rev: Option<String>,
    pub kind: String,
    pub optional: bool,
    pub default_features: bool,
    pub features: Vec<String>,
    pub target: Option<String>,
}

impl Dependency {
    pub fn key(&self) -> &str {
        self.rename.as_deref().unwrap_or(&self.name)
    }
}

/// Where the harness finds `roundhouse`.
#[derive(Clone, Debug)]
pub struct RoundhouseDependency {
    pub path: Option<PathBuf>,
    pub version: String,
}
