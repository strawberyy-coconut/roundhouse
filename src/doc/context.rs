//! The document build's context and component ledger.
//!
//! The ledger used to live in a thread-local so `ApiSchema::register` could claim a component
//! name without receiving extra arguments. It is now an explicit value: the build owns one
//! [`DocContext`], passes it to every schema/document call, and reads the collisions at the end.
//! A schema can therefore be registered outside a document build by passing its `Components`
//! directly; that context has no ledger and every claim is inactive.

use std::collections::HashMap;

use utoipa::openapi::schema::Components;

use crate::build_error::Source;
use crate::doc::registration::Collision;
use roundhouse_schema::{Claim, SchemaContext};

/// One document build's component table, name ledger and current source location.
pub struct DocContext<'a> {
    components: &'a mut Components,
    names: HashMap<String, &'static str>,
    collisions: Vec<Collision>,
    source: Source,
}

impl<'a> DocContext<'a> {
    pub fn new(components: &'a mut Components) -> Self {
        Self {
            components,
            names: HashMap::new(),
            collisions: Vec::new(),
            source: Source::new("<app>", "<app>", 0),
        }
    }

    /// Associate subsequent registrations with the route currently being documented.
    pub fn set_source(&mut self, source: Source) {
        self.source = source;
    }

    /// Take the collisions recorded during this build.
    pub fn take_collisions(&mut self) -> Vec<Collision> {
        std::mem::take(&mut self.collisions)
    }

    fn claim_component(&mut self, name: &str, type_name: &'static str) -> Claim {
        if let Some(existing) = self.names.get(name) {
            if *existing == type_name {
                return Claim::Already;
            }
            let first = *existing;
            self.record_collision(name, first, type_name);
            return Claim::Collision;
        }

        // A component can exist without going through this ledger (a manual insert or a
        // `ToSchema::schemas` entry registered by a nested type). Keep the first one and report
        // the second type; the first type is unknown in that case.
        if self.components.schemas.contains_key(name) {
            self.record_collision(name, "<unregistered component>", type_name);
            self.names.insert(name.to_string(), type_name);
            return Claim::Collision;
        }

        self.names.insert(name.to_string(), type_name);
        Claim::Claimed
    }

    fn record_component_collision(
        &mut self,
        name: &str,
        first: &'static str,
        second: &'static str,
    ) {
        self.collisions.push(Collision {
            name: name.to_string(),
            first_type: first,
            second_type: second,
            source: self.source,
        });
    }
}

impl SchemaContext for DocContext<'_> {
    fn components(&mut self) -> &mut Components {
        self.components
    }

    fn claim(&mut self, name: &str, type_name: &'static str) -> Claim {
        self.claim_component(name, type_name)
    }

    fn record_collision(&mut self, name: &str, first: &'static str, second: &'static str) {
        self.record_component_collision(name, first, second);
    }
}
