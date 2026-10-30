use axum::Router;

use crate::registry::Module;

/// An explicit group of route modules emitted by `routes!`.
///
/// A `routes!` invocation produces one [`Module`] and a `__roundhouse_fragment()` function
/// returning this value. `...module` spreads compose fragments, and [`crate::App::fragment`] roots the
/// app at one. The fragment owns the modules directly; there is no link-time discovery.
#[derive(Debug, Clone, Default)]
pub struct Fragment {
    roots: Vec<&'static Module>,
}

impl Fragment {
    /// Start a fragment at one module.
    pub fn new(module: &'static Module) -> Self {
        Self {
            roots: vec![module],
        }
    }

    /// Add every root of `other` to this fragment.
    pub fn include(mut self, other: Self) -> Self {
        self.roots.extend(other.roots);
        self
    }

    /// Add one module root.
    pub fn with(mut self, module: &'static Module) -> Self {
        self.roots.push(module);
        self
    }

    /// The root modules in composition order.
    pub fn roots(&self) -> &[&'static Module] {
        &self.roots
    }
}

/// A hand-written router merged with `App::merge`, plus where it was merged.
pub(crate) struct PendingMerge {
    pub(crate) router: Router<()>,
    pub(crate) source: crate::build_error::Source,
}

/// One entry in a built app's merge ledger.
///
/// A merged router is served but not discovered for the document, so the document is incomplete.
/// An application can assert its own migration debt with [`crate::Built::merges`] in CI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeRecord {
    pub(crate) source: crate::build_error::Source,
}

impl MergeRecord {
    pub fn source(&self) -> crate::build_error::Source {
        self.source
    }

    pub fn location(&self) -> String {
        format!("{}:{}", self.source.file, self.source.line)
    }
}

/// Whether extractor rejection responses are added to the document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DocumentRejections {
    /// Add every extractor failure (`400`/`413`/`415`, auth `401`/`403`). The default.
    #[default]
    Auto,
    /// Publish only the responses the handler and its declarations supply.
    None,
}

impl From<bool> for DocumentRejections {
    fn from(enabled: bool) -> Self {
        if enabled {
            DocumentRejections::Auto
        } else {
            DocumentRejections::None
        }
    }
}
