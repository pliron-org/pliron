//! An [Op] is a thin wrapper arround an [Operation], providing
//! API specific to the [OpId] of that Operation.
//!
//! See MLIR's [Op](https://mlir.llvm.org/docs/Tutorials/Toy/Ch-2/#op-vs-operation-using-mlir-operations).
//!
//! New [Op]s can be easily declared using the [def_op](pliron::derive::def_op)
//! proc macro from the pliron-derive crate.
//!
//! Common semantics, API and behaviour of [Op]s are
//! abstracted into Op interfaces. Interfaces in pliron capture MLIR
//! functionality of both [Traits](https://mlir.llvm.org/docs/Traits/)
//! and [Interfaces](https://mlir.llvm.org/docs/Interfaces/).
//! Interfaces must all implement an associated function named `verify` with
//! the type [OpInterfaceVerifier].
//!
//! Interfaces are rust Trait definitions annotated with the attribute macro
//! [op_interface](pliron::derive::op_interface). The attribute ensures that any
//! verifiers of super-interfaces are run prior to the verifier of this interface.
//! Note: Super-interface verifiers *may* run multiple times for the same op.
//!
//! [Op]s that implement an interface must annotate the implementation with
//! [op_interface_impl](pliron::derive::op_interface_impl) macro to ensure that
//! the interface verifier is automatically called during verification
//! and that a `&dyn Op` object can be [cast](op_cast) into an interface object,
//! (or that it can be checked if the interface is [implemented](op_impls))
//! with ease.
//!
//! Use [verify_op] or [verify_operation] to verify an [Op] object or [Operation].
//! The verifier methods of [Op] or [Operation] may not, by themselves do a complete verification.
//!
//! [OpObj]s can be downcasted to their concrete types using
//! [downcast_rs](https://docs.rs/downcast-rs/latest/downcast_rs/#example-without-generics).

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    fmt::{self, Display},
    hash::Hash,
    ops::Deref,
};
use downcast_rs::{Downcast, impl_downcast};
use dyn_clone::DynClone;
use thiserror::Error;

use crate::{
    attribute::AttributeDict,
    combine::{
        Parser,
        parser::{self, char::spaces},
        token,
    },
    common_traits::{Named, Verify},
    context::{Context, Ptr, collect_deduped_interface_verifiers},
    dialect::{Dialect, DialectName},
    identifier::Identifier,
    impl_printable_for_display, input_err,
    irfmt::{
        parsers::{
            block_opd_parser, delimited_list_parser, location, process_parsed_ssa_defs, spaced,
            ssa_opd_parser, zero_or_more_parser,
        },
        printers::iter_with_sep,
    },
    location::{Located, Location},
    operation::{Operation, verify_operation},
    parsable::{IntoParseResult, Parsable, ParseResult, StateStream},
    printable::{self, Printable},
    region::Region,
    result::Result,
    std_deps::{hash::FxHashMap, sync::LazyLock},
    r#type::{TypeSig, Typed},
};

#[derive(Clone, Hash, PartialEq, Eq)]
/// An Op's name (not including it's dialect).
pub struct OpName(String);

impl OpName {
    /// Create a new OpName.
    pub fn new(name: &str) -> OpName {
        OpName(name.to_string())
    }
}

impl Deref for OpName {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl_printable_for_display!(OpName);

impl Parsable for OpName {
    type Arg = ();
    type Parsed = OpName;

    fn parse<'a>(
        state_stream: &mut crate::parsable::StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed>
    where
        Self: Sized,
    {
        Identifier::parser(())
            .map(|name| OpName::new(&name))
            .parse_stream(state_stream)
            .into()
    }
}

impl Display for OpName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A combination of an [Op]'s name and its dialect.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct OpId {
    pub dialect: DialectName,
    pub name: OpName,
}

impl_printable_for_display!(OpId);

impl Parsable for OpId {
    type Arg = ();
    type Parsed = OpId;

    // Parses (but does not validate) a OpId.
    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed>
    where
        Self: Sized,
    {
        let mut parser = DialectName::parser(())
            .skip(parser::char::char('.'))
            .and(OpName::parser(()))
            .map(|(dialect, name)| OpId { dialect, name });
        parser.parse_stream(state_stream).into()
    }
}

impl Display for OpId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}", self.dialect, self.name)
    }
}

pub(crate) type ConcreteOpInfo = (fn(Ptr<Operation>) -> OpObj, core::any::TypeId);

/// A wrapper around [Operation] for Op(code) specific work.
/// All per-instance data must be in the underyling Operation,
/// which means that [OpObj]s are light-weight.
///
/// See [module](crate::op) documentation for more information.
pub trait Op: Downcast + Verify + Printable + DynClone {
    /// Get the underlying IR Operation
    fn get_operation(&self) -> Ptr<Operation>;

    #[doc(hidden)]
    /// Create a new [OpObj], by boxing [Op].
    ///
    /// **WARNING**: Does not check that the operation is of the correct OpId.
    fn wrap_operation(op: Ptr<Operation>) -> OpObj
    where
        Self: Sized;

    #[doc(hidden)]
    /// Create a concrete [Op] from an [Operation].
    ///
    /// **WARNING**: Does not check that the operation is of the correct OpId.
    fn from_operation(op: Ptr<Operation>) -> Self
    where
        Self: Sized;

    /// Get details about the concrete Op type.
    fn get_concrete_op_info() -> ConcreteOpInfo
    where
        Self: Sized,
    {
        (Self::wrap_operation, core::any::TypeId::of::<Self>())
    }

    /// Get this Op's OpId
    fn get_opid(&self) -> OpId;

    /// Get this Op's OpId, without self reference.
    fn get_opid_static() -> OpId
    where
        Self: Sized;

    #[doc(hidden)]
    /// Verify all interfaces implemented by this op.
    fn verify_interfaces(&self, ctx: &Context) -> Result<()>;

    /// Register Op in Context and add it to its dialect.
    fn register(ctx: &mut Context)
    where
        Self: Sized + Parsable<Arg = Vec<(Identifier, Location)>, Parsed = OpBox>,
    {
        let op_parser: OpParserFn = Box::new(|&(), args| Self::parser(args));
        let opid = Self::get_opid_static();
        Dialect::register(ctx, &opid.dialect).add_op(opid.clone(), op_parser);
    }

    /// Get Op's location
    fn loc(&self, ctx: &Context) -> Location {
        self.get_operation().deref(ctx).loc()
    }
}
impl_downcast!(Op);
dyn_clone::clone_trait_object!(Op);

/// A storable function pointer to parse a specific [Op].
/// The [Op]'s [Dialect] maps an [OpId] to such a parser.
pub(crate) type OpParserFn = Box<
    for<'a> fn(
        &'a (),
        Vec<(Identifier, Location)>,
    ) -> Box<dyn Parser<StateStream<'a>, Output = OpObj, PartialState = ()> + 'a>,
>;

/// [Op] objects are boxed and stored in the IR.
pub type OpObj = OpBox;

impl PartialEq for OpObj {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref()
            .get_operation()
            .eq(&other.as_ref().get_operation())
    }
}

/// Verify an [Op] object and all its nested regions and blocks.
pub fn verify_op(op: &dyn Op, ctx: &Context) -> Result<()> {
    verify_operation(op.get_operation(), ctx)
}

impl Eq for OpObj {}

impl Hash for OpObj {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_ref().get_operation().hash(state)
    }
}

/// Marker trait for op interface trait objects.
///
/// This is auto-implemented by the `#[op_interface]` macro for `dyn Interface`
/// objects and is used to restrict [op_cast] and [op_impls] to interface casts.
#[diagnostic::on_unimplemented(
    message = "`{Self}` not an op interface.",
    label = "If `{Self}` is a trait, annotate it with #[op_interface] to be able to cast to it from a `&dyn Op`",
    note = "If you want to cast to a concrete `Op`, use `downcast_ref` instead."
)]
pub trait OpInterfaceMarker {}

/// Cast reference to an [Op] object to an interface reference.
///
/// Right usage: cast to an interface trait object.
/// ```
/// use pliron::builtin::op_interfaces::SymbolTableInterface;
/// use pliron::op::{Op, op_cast};
///
/// fn right_cast(op: &dyn Op) {
///     let _ = op_cast::<dyn SymbolTableInterface>(op);
/// }
/// ```
///
/// Casting to concrete [Op] types are intentionally rejected.
/// ```compile_fail
/// use pliron::builtin::ops::ModuleOp;
/// use pliron::op::{Op, op_cast};
///
/// fn wrong_cast(op: &dyn Op) {
///     let _ = op_cast::<ModuleOp>(op);
/// }
/// ```
/// Use [downcast_rs](https://docs.rs/downcast-rs/latest/downcast_rs/#example-without-generics)
/// to downcast to concrete [Op] types.
pub fn op_cast<T: ?Sized + OpInterfaceMarker + 'static>(op: &dyn Op) -> Option<&T> {
    crate::utils::trait_cast::any_to_trait::<T>(op.as_any())
}

/// Does this [Op] object implement interface `T`?
///
/// Right usage: query using an interface trait object.
/// ```
/// use pliron::builtin::op_interfaces::SymbolTableInterface;
/// use pliron::op::{Op, op_impls};
///
/// fn right_query(op: &dyn Op) {
///     let _ = op_impls::<dyn SymbolTableInterface>(op);
/// }
/// ```
///
/// Querying with a concrete [Op] type is intentionally rejected.
/// ```compile_fail
/// use pliron::builtin::ops::ModuleOp;
/// use pliron::op::{Op, op_impls};
///
/// fn wrong_query(op: &dyn Op) {
///     let _ = op_impls::<ModuleOp>(op);
/// }
/// ```
pub fn op_impls<T: ?Sized + OpInterfaceMarker + 'static>(op: &dyn Op) -> bool {
    op_cast::<T>(op).is_some()
}

/// Every op interface must have a function named `verify` with this type.
pub type OpInterfaceVerifier = fn(&dyn Op, &Context) -> Result<()>;
/// Function returns the list of super verifiers, followed by a self verifier, for an interface.
pub type OpInterfaceAllVerifiers = fn() -> Vec<OpInterfaceVerifier>;

#[doc(hidden)]
/// An [Op] paired with an interface it implements
/// (specifically the verifiers (including super verifiers) for that interface).
type OpInterfaceVerifierInfo = (core::any::TypeId, OpInterfaceAllVerifiers);

#[cfg(not(target_family = "wasm"))]
pub mod statics {
    use super::*;

    #[::pliron::linkme::distributed_slice]
    pub static OP_INTERFACE_VERIFIERS: [LazyLock<OpInterfaceVerifierInfo>] = [..];

    pub fn get_op_interface_verifiers()
    -> impl Iterator<Item = &'static LazyLock<OpInterfaceVerifierInfo>> {
        OP_INTERFACE_VERIFIERS.iter()
    }
}

#[cfg(target_family = "wasm")]
pub mod statics {
    use super::*;
    use crate::utils::inventory::LazyLockWrapper;

    ::pliron::inventory::collect!(LazyLockWrapper<OpInterfaceVerifierInfo>);

    pub fn get_op_interface_verifiers()
    -> impl Iterator<Item = &'static LazyLock<OpInterfaceVerifierInfo>> {
        ::pliron::inventory::iter::<LazyLockWrapper<OpInterfaceVerifierInfo>>().map(|llw| llw.0)
    }
}

pub use statics::*;

#[doc(hidden)]
/// A map from every [Op] to its ordered (as per interface deps) list of interface verifiers.
/// An interface's super-interfaces are to be verified before it itself is.
pub static OP_INTERFACE_VERIFIERS_MAP: LazyLock<
    FxHashMap<core::any::TypeId, Vec<OpInterfaceVerifier>>,
> = LazyLock::new(|| collect_deduped_interface_verifiers(get_op_interface_verifiers()));

/// Printer for an [Op] in canonical syntax.
/// `res_1, res_2, ... res_n =
///      op_id (opd_1, opd_2, ... opd_n) [succ_1, succ_2, ... succ_n] [attr-dict]: function-type (regions)*`
pub fn canonical_syntax_print(
    op: OpObj,
    ctx: &Context,
    state: &printable::State,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    let sep = printable::ListSeparator::CharSpace(',');
    let opid = op.as_ref().get_opid();
    let op = op.as_ref().get_operation().deref(ctx);
    let operands = iter_with_sep(op.operands(), sep);
    let successors = iter_with_sep(
        op.successors()
            .map(|succ| "^".to_string() + &succ.unique_name(ctx)),
        sep,
    );
    let op_type = TypeSig {
        arguments: op.operands().map(|opd| opd.get_type(ctx)).collect(),
        results: op.results().map(|res| res.get_type(ctx)).collect(),
    };
    let regions = iter_with_sep(op.regions(), printable::ListSeparator::Newline);

    if op.get_num_results() != 0 {
        let results = iter_with_sep(op.results(), sep);
        write!(f, "{} = ", results.disp(ctx))?;
    }

    write!(
        f,
        "{} ({}) [{}] {}: <{}>",
        opid.disp(ctx),
        operands.disp(ctx),
        successors.disp(ctx),
        op.attributes.clone_skip_outlined().disp(ctx),
        op_type.disp(ctx),
    )?;

    if op.num_regions() > 0 {
        regions.fmt(ctx, state, f)?;
    }
    Ok(())
}

#[derive(Error, Debug)]
pub enum CanonicalSyntaxParseError {
    #[error("Type specifies {num_res_ty} results, but operation has {num_res} results")]
    ResultsMismatch { num_res_ty: usize, num_res: usize },
    #[error("Type specifies {num_opd_ty} operands, but operation has {num_opd} operands")]
    OperandsMismatch { num_opd_ty: usize, num_opd: usize },
}

/// Parse an [Op] in canonical syntax.
/// See [canonical_syntax_print] for the syntax.
pub fn canonical_syntax_parse<'a, T: Op>(
    state_stream: &mut StateStream<'a>,
    results: Vec<(Identifier, Location)>,
) -> ParseResult<'a, OpObj> {
    // Results and opid have already been parsed. Continue after that.
    let mut without_regions = delimited_list_parser('(', ')', ',', ssa_opd_parser())
        .and(spaces().with(delimited_list_parser('[', ']', ',', block_opd_parser())))
        .and(spaces().with(AttributeDict::parser(())))
        .skip(spaced(token(':')))
        .and((
            location(),
            token('<')
                .with(TypeSig::parser(()))
                .skip(spaced(token('>'))),
        ))
        .then(
            move |(((operands, successors), attr_dict), (fty_loc, fty))| {
                let results = results.clone();
                let fty_loc = fty_loc.clone();
                combine::parser(move |parsable_state: &mut StateStream<'a>| {
                    let results = results.clone();
                    let ctx = &mut parsable_state.state.ctx;
                    if fty.results.len() != results.len() {
                        input_err!(
                            fty_loc.clone(),
                            CanonicalSyntaxParseError::ResultsMismatch {
                                num_res_ty: fty.results.len(),
                                num_res: results.len()
                            }
                        )?
                    }
                    if operands.len() != fty.arguments.len() {
                        input_err!(
                            fty_loc.clone(),
                            CanonicalSyntaxParseError::OperandsMismatch {
                                num_opd_ty: fty.arguments.len(),
                                num_opd: operands.len()
                            }
                        )?
                    }
                    let opr = Operation::new(
                        ctx,
                        T::get_concrete_op_info(),
                        fty.results.clone(),
                        operands.clone(),
                        successors.clone(),
                        0,
                    );
                    opr.deref_mut(ctx).attributes = attr_dict.clone();
                    process_parsed_ssa_defs(parsable_state, &results, opr)?;
                    Ok(opr).into_parse_result()
                })
            },
        );

    let op = without_regions.parse_stream(state_stream).into_result()?.0;
    zero_or_more_parser(Region::parser(op))
        .parse_stream(state_stream)
        .into_result()?;
    let op = T::wrap_operation(op);
    Ok(op).into_parse_result()
}

/// Parser for an [Op] in canonical syntax.
/// See [canonical_syntax_print] for the syntax.
pub fn canonical_syntax_parser<'a, T: Op>(
    results: Vec<(Identifier, Location)>,
) -> Box<dyn Parser<StateStream<'a>, Output = OpObj, PartialState = ()> + 'a> {
    combine::parser(move |parsable_state: &mut StateStream<'a>| {
        canonical_syntax_parse::<T>(parsable_state, results.clone())
    })
    .boxed()
}

/// This must always be the same as any concrete [Op] object.
#[derive(Clone)]
struct OpData {
    #[allow(unused)]
    op: Ptr<Operation>,
}

/// A stack allocated alternative to [Box] for [Op] objects.
#[derive(Clone)]
pub struct OpBox {
    data: OpData,
    vtable_ptr: *const (),
}

impl OpBox {
    /// Create a new [OpBox] from a concrete [Op] object.
    pub fn new<T: Op>(op: T) -> Self {
        /// Static assertion to ensure that concrete [Op]s
        /// always are the same as our [OpData] struct.
        struct StaticAsserter<S>(S);
        impl<S> StaticAsserter<S> {
            const ASSERTTION: () = {
                // Ensure that OpData and T have the same size.
                assert!(
                    core::mem::size_of::<OpData>() == core::mem::size_of::<S>(),
                    "OpBox can only box Op objects"
                );
            };
        }
        let _: () = StaticAsserter::<T>::ASSERTTION;

        let dyn_ref: &dyn Op = &op;
        let (_, vtable_ptr) =
            unsafe { core::mem::transmute::<&dyn Op, (*const T, *const ())>(dyn_ref) };

        OpBox {
            data: OpData {
                op: op.get_operation(),
            },
            vtable_ptr,
        }
    }

    /// Get a reference to the underlying [Op] object.
    pub fn op_ref(&self) -> &dyn Op {
        unsafe {
            let dyn_ref: &dyn Op = core::mem::transmute::<(&OpData, *const ()), &dyn Op>((
                &self.data,
                self.vtable_ptr,
            ));
            dyn_ref
        }
    }

    /// Downcast this [OpBox] to a concrete [Op] type.
    pub fn downcast<T: Op>(self) -> Option<T> {
        self.as_ref()
            .downcast_ref::<T>()
            .map(|op| T::from_operation(op.get_operation()))
    }
}

impl AsRef<dyn Op> for OpBox {
    fn as_ref(&self) -> &dyn Op {
        self.op_ref()
    }
}

impl Deref for OpBox {
    type Target = dyn Op;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}
