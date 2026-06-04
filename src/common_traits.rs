//! Utility traits such as [Named], [Verify] etc.

use crate::{
    context::Context,
    identifier::{Identifier, underscore},
    result::Result,
};

/// Check and ensure correctness.
pub trait Verify {
    fn verify(&self, ctx: &Context) -> Result<()>;
}

/// Sugar to implement a verifier that always succeeds.
/// Usage:
/// ```
/// use pliron::{context::Context, impl_verify_succ, common_traits::Verify};
/// struct A;
/// let a = A;
/// let ctx = Context::new();
/// assert!(a.verify(&ctx).is_ok());
/// impl_verify_succ!(A);
/// ```
#[deprecated(
    since = "0.14.0",
    note = "Consider using `pliron::derive::verify_succ` instead"
)]
#[macro_export]
macro_rules! impl_verify_succ {
    ($op_name:path) => {
        impl $crate::common_traits::Verify for $op_name {
            fn verify(&self, _ctx: &$crate::context::Context) -> $crate::result::Result<()> {
                Ok(())
            }
        }
    };
}

/// Anything that has a name.
pub trait Named {
    // A (not necessarily unique) name.
    fn given_name(&self, ctx: &Context) -> Option<Identifier>;
    // A Unique (within the context) ID.
    fn id(&self, ctx: &Context) -> Identifier;
    // A unique name; concatenation of name and id.
    fn unique_name(&self, ctx: &Context) -> Identifier {
        match self.given_name(ctx) {
            Some(given_name) => given_name + underscore() + self.id(ctx),
            None => self.id(ctx),
        }
    }
}

/// For reference-counted containers, [share](Self::share) data by increasing the reference count.
/// This is equivalent in semantics to (i.e., [Rc::clone](alloc::rc::Rc::clone)),
/// but with a goal of having a less ambiguous name.
pub trait RcShare {
    /// Share this object with someone else by increasing the reference count.
    fn share(&self) -> Self;
}
