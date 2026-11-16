//! The concrete-type bridge between `#[handler]` and [`crate::arg::ExtractArg`].
//!
//! The macro sees only the token `T` of an argument. This probe resolves at the concrete type:
//! when `T` implements `ExtractArg`, route-level validation runs; otherwise the argument is
//! skipped. The fallback keeps a custom or foreign extractor compiling without a schema
//! implementation.

use crate::arg::ExtractArg;
use crate::doc::ValidateHook;
use crate::extract::ValidationContext;
use crate::problem::Problem;

/// Autoref probe for the runtime validation hook.
///
/// Field validation calls this so a type without `validate_with` hooks still compiles; the
/// fallback impl does nothing.
pub struct HookProbe<T: ?Sized>(std::marker::PhantomData<T>);

impl<T: ?Sized> HookProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ?Sized> Default for HookProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeHook<T: ?Sized> {
    fn probe_validate_at(
        self,
        value: &T,
        pointer: &str,
        errors: &mut crate::validate::ValidationErrors,
    );
}

impl<T: ValidateHook + ?Sized> ProbeHook<T> for &HookProbe<T> {
    fn probe_validate_at(
        self,
        value: &T,
        pointer: &str,
        errors: &mut crate::validate::ValidationErrors,
    ) {
        ValidateHook::validate_at(value, pointer, errors);
    }
}

impl<T: ?Sized> ProbeHook<T> for &&HookProbe<T> {
    fn probe_validate_at(
        self,
        _value: &T,
        _pointer: &str,
        _errors: &mut crate::validate::ValidationErrors,
    ) {
    }
}

pub struct ArgProbe<T>(std::marker::PhantomData<T>);

impl<T> ArgProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ArgProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeArg<T> {
    #[allow(clippy::result_large_err)]
    fn validate(self, value: &T, context: &ValidationContext) -> Result<(), Problem>;
    fn body_arg(self) -> bool;
    fn verified(self) -> bool;
}

impl<T: ExtractArg> ProbeArg<T> for &ArgProbe<T> {
    fn validate(self, value: &T, context: &ValidationContext) -> Result<(), Problem> {
        value.validate(context)
    }

    fn body_arg(self) -> bool {
        T::IS_BODY
    }

    fn verified(self) -> bool {
        T::SHIM_VALIDATES
    }
}

impl<T> ProbeArg<T> for &&ArgProbe<T> {
    fn validate(self, _value: &T, _context: &ValidationContext) -> Result<(), Problem> {
        Ok(())
    }

    fn body_arg(self) -> bool {
        false
    }

    fn verified(self) -> bool {
        false
    }
}
