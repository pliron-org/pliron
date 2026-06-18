use crate::{
    context::Context,
    result::Result,
    r#type::{Type, TypeHandle},
    utils::apfloat::Semantics,
};
use alloc::vec::Vec;
use pliron_derive::type_interface;

/// Types that represent floating point numbers.
#[type_interface]
pub trait FloatTypeInterface {
    /// Returns the semantics of the float type.
    fn get_semantics(&self) -> Semantics;
    fn verify(_attr: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// Types that represent a function, i.e., take in arguments and return results.
#[type_interface]
pub trait FunctionTypeInterface {
    /// Returns the argument types of the function type.
    fn arg_types(&self) -> Vec<TypeHandle>;
    /// Returns the result types of the function type.
    fn res_types(&self) -> Vec<TypeHandle>;
    fn verify(_ty: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}
