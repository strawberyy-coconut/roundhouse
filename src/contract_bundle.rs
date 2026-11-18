//! The runtime contract artifact.
//!
//! This module deliberately has no `utoipa` dependency: a production build enables
//! `contract` to validate requests from the committed `contract.json` and serve
//! `openapi.json` verbatim. Documentation generation enables the heavier `build` feature.

/// The artifact kind written by `Contract::contract_json`.
pub(crate) const CONTRACT_KIND: &str = "roundhouse.contract";
/// The only contract layout this runtime understands.
///
/// Bump this when the artifact loses or changes a field the runtime reads. A new runtime accepts
/// only its own version; users regenerate with `cargo roundhouse prepare`.
pub(crate) const CONTRACT_VERSION: u64 = 1;

/// The two build-time artifacts embedded by [`include_contract!`](crate::include_contract).
///
/// `contract` is the runtime validation/metadata artifact; `openapi` is the document served at
/// `/openapi.json`. They are generated together by `cargo roundhouse prepare`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContractBundle {
    /// The contents of `.roundhouse/contract.json`.
    pub contract: &'static str,
    /// The contents of `.roundhouse/openapi.json`.
    pub openapi: &'static str,
}

impl ContractBundle {
    /// Construct a bundle from the two embedded artifacts.
    pub const fn new(contract: &'static str, openapi: &'static str) -> Self {
        Self { contract, openapi }
    }

    /// Check the artifact can be consumed by this runtime.
    pub(crate) fn validate(&self) -> Result<(), String> {
        let contract: serde_json::Value = serde_json::from_str(self.contract)
            .map_err(|error| format!("contract.json is not valid JSON: {error}"))?;
        let kind = contract["kind"].as_str().unwrap_or_default();
        if kind != CONTRACT_KIND {
            return Err(format!(
                "contract.json has kind `{kind}`, expected `{CONTRACT_KIND}`; regenerate it with \
                 `cargo roundhouse prepare`"
            ));
        }
        let version = contract["contract_version"].as_u64().unwrap_or_default();
        if version != CONTRACT_VERSION {
            return Err(format!(
                "contract.json uses contract version {version}, but this roundhouse runtime reads \
                 version {CONTRACT_VERSION}; regenerate it with `cargo roundhouse prepare`"
            ));
        }
        if contract["operations"].as_array().is_none() {
            return Err("contract.json has no operation table".to_string());
        }
        serde_json::from_str::<serde_json::Value>(self.openapi)
            .map_err(|error| format!("openapi.json is not valid JSON: {error}"))?;
        Ok(())
    }

    /// Compile a test-only response validator from the contract response table.
    #[cfg(all(feature = "build", feature = "validation"))]
    pub fn response_validator(
        &self,
    ) -> Result<crate::ContractResponseValidator, roundhouse_schema::engine::SchemaError> {
        crate::ContractResponseValidator::from_contract_json(self.contract)
    }
}
