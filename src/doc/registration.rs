//! A build-scoped ledger of component names.
//!
//! `Components` stores a schema by name but not the Rust type that produced it, so two types that
//! share a name silently keep the first registration. The ledger records the type name for the
//! duration of one document build and turns a second, different type into a build error.

use std::cell::RefCell;
use std::collections::HashMap;

use utoipa::openapi::schema::Components;

use crate::build_error::Source;

/// A component name claimed by two different Rust types.
#[derive(Clone, Debug)]
pub struct Collision {
    pub name: String,
    pub first_type: &'static str,
    pub second_type: &'static str,
    pub source: Source,
}

/// The result of claiming a component name.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// The name was new and is now owned by this type.
    Claimed,
    /// This type (or its recursive registration) already claimed the name.
    Already,
    /// Another type owns the name. The caller must not overwrite the first schema.
    Collision,
    /// No document build is active; insert normally.
    Inactive,
}

#[derive(Default)]
struct Ledger {
    names: HashMap<String, &'static str>,
    collisions: Vec<Collision>,
    source: Option<Source>,
}

thread_local! {
    static LEDGER: RefCell<Option<Ledger>> = const { RefCell::new(None) };
}

/// Starts a document build's registration pass and clears it on every exit path.
#[doc(hidden)]
pub struct Guard(());

impl Guard {
    pub fn new() -> Self {
        begin();
        Self(())
    }
}

impl Default for Guard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = finish();
    }
}

/// Start a document build's registration pass.
pub fn begin() {
    LEDGER.with(|ledger| *ledger.borrow_mut() = Some(Ledger::default()));
}

/// Associate subsequent registrations with the route currently being documented.
pub fn set_source(source: Source) {
    LEDGER.with(|ledger| {
        if let Some(ledger) = ledger.borrow_mut().as_mut() {
            ledger.source = Some(source);
        }
    });
}

/// Finish the pass and return every collision it saw.
pub fn finish() -> Vec<Collision> {
    LEDGER.with(|ledger| {
        ledger
            .borrow_mut()
            .take()
            .map(|ledger| ledger.collisions)
            .unwrap_or_default()
    })
}

/// Claim a component name for `T`.
#[doc(hidden)]
pub fn claim<T: ?Sized>(name: &str, components: &Components) -> Claim {
    claim_name(name, std::any::type_name::<T>(), components)
}

/// Claim a component name for a type that has no usable `Self` at the call site.
#[doc(hidden)]
pub fn claim_name(name: &str, type_name: &'static str, components: &Components) -> Claim {
    LEDGER.with(|ledger| {
        let mut slot = ledger.borrow_mut();
        let Some(ledger) = slot.as_mut() else {
            return Claim::Inactive;
        };

        if let Some(existing) = ledger.names.get(name) {
            if *existing == type_name {
                return Claim::Already;
            }
            record(ledger, name, existing, type_name);
            return Claim::Collision;
        }

        // A component can exist without going through this ledger (a manual insert or a
        // `ToSchema::schemas` entry registered by a nested type). Keep the first one and report
        // the second type; the first type is unknown in that case.
        if components.schemas.contains_key(name) {
            record(ledger, name, "<unregistered component>", type_name);
            ledger.names.insert(name.to_string(), type_name);
            return Claim::Collision;
        }

        ledger.names.insert(name.to_string(), type_name);
        Claim::Claimed
    })
}

/// Record a structural collision for a component whose Rust identity is unknown.
#[doc(hidden)]
pub fn record_collision(name: &str, first: &'static str, second: &'static str) {
    LEDGER.with(|ledger| {
        let mut slot = ledger.borrow_mut();
        if let Some(ledger) = slot.as_mut() {
            record(ledger, name, first, second);
        }
    });
}

fn record(ledger: &mut Ledger, name: &str, first: &'static str, second: &'static str) {
    ledger.collisions.push(Collision {
        name: name.to_string(),
        first_type: first,
        second_type: second,
        source: ledger
            .source
            .unwrap_or_else(|| Source::new("<app>", "<app>", 0)),
    });
}
