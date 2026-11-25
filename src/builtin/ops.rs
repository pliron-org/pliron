// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Builtin dialect ops

use super::{
    attr_interfaces::TypedAttrInterface,
    attributes::{IdentifierAttr, TypeAttr},
    op_interfaces::{
        self, AtLeastNOpdsInterface, CallOpCallable, CallOpInterface, IsTerminatorInterface,
        IsolatedFromAboveInterface, NOpdsInterface, OneRegionInterface, OneResultInterface,
        SingleBlockRegionInterface, SymbolOpInterface, SymbolTableInterface, SymbolUserOpInterface,
        verify_func_symbol_use, verify_types_match,
    },
    types::{FunctionType, UnitType},
};
use crate::{
    attribute::{Attribute, AttributeDict, attr_cast},
    basic_block::BasicBlock,
    builtin::{
        op_interfaces::{
            ATTR_KEY_SYM_NAME, NResultsInterface, NoTerminatorInterface, RegionKind,
            RegionKindInterface,
        },
        ops::func_op_attr_names::ATTR_KEY_BUILTIN_FUNC_TYPE,
        type_interfaces::FunctionTypeInterface,
    },
    combine::{Parser, optional, token},
    common_traits::{Named, Verify},
    context::{Context, Ptr},
    ident,
    identifier::Identifier,
    input_err,
    irfmt::{
        parsers::{
            delimited_list_parser, process_parsed_ssa_defs, spaced, ssa_opd_parser, type_parser,
        },
        printers::{
            list_with_sep,
            op::{region, symb_op_header, typed_symb_op_header},
        },
    },
    linked_list::ContainsLinkedList,
    location::Location,
    op::{Op, OpObj},
    operation::Operation,
    parsable::{IntoParseResult, Parsable, ParseResult, StateStream, StreamLocation},
    printable::{self, ListSeparator, Printable, indented_nl},
    region::Region,
    result::Result,
    symbol_table::SymbolTableCollection,
    r#type::{TypeHandle, TypeInterfaceHandle, TypeSig, Typed, TypedHandle},
    value::Value,
    verify_err,
};
use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::{cell::Ref, ops::Range};
use pliron::derive::{op_interface_impl, pliron_op};
use thiserror::Error;

/// Represents a module, a top level container operation.
///
/// It contains a single [Graph](super::op_interfaces::RegionKind::Graph)
/// region containing a single block which can contain any operations and
/// does not have a terminator.
///
#[pliron_op(
    name = "builtin.module",
    interfaces = [
        OneRegionInterface,
        SingleBlockRegionInterface,
        SymbolTableInterface,
        SymbolOpInterface,
        IsolatedFromAboveInterface,
        NOpdsInterface<0>,
        NResultsInterface<0>,
        NoTerminatorInterface
    ],
    verifier = "succ",
)]
pub struct ModuleOp;

#[op_interface_impl]
impl RegionKindInterface for ModuleOp {
    fn get_region_kind(&self, _idx: usize) -> RegionKind {
        RegionKind::Graph
    }
}

impl Printable for ModuleOp {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        symb_op_header(self).fmt(ctx, state, f)?;
        write!(f, " ")?;
        let mut attributes_to_print_separately =
            self.op.deref(ctx).attributes.clone_skip_outlined(ctx);
        attributes_to_print_separately
            .0
            .retain(|key, _| key != &ATTR_KEY_SYM_NAME);
        if !attributes_to_print_separately.0.is_empty() {
            let _indent = state.indent();
            write!(f, "{}", indented_nl(state))?;
            attributes_to_print_separately.fmt(ctx, state, f)?;
        }
        region(self).fmt(ctx, state, f)?;
        Ok(())
    }
}

impl Parsable for ModuleOp {
    type Arg = Vec<(Identifier, Location)>;
    type Parsed = OpObj;
    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        results: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        if !results.is_empty() {
            input_err!(
                state_stream.loc(),
                op_interfaces::NResultsVerifyErr(0, results.len())
            )?
        }
        let opop = Operation::new_op::<Self>(state_stream.state.ctx, vec![], vec![], vec![], 0);
        let op = opop.get_operation();
        let mut parser = (
            spaced(token('@').with(Identifier::parser(()))),
            spaced(optional(AttributeDict::parser(()))),
            spaced(Region::parser(op)),
        );
        parser
            .parse_stream(state_stream)
            .map(|(name, attributes, _region)| -> OpObj {
                let ctx = &mut state_stream.state.ctx;
                op.deref_mut(ctx).attributes = attributes.unwrap_or_default();
                opop.set_symbol_name(ctx, name);
                OpObj::new(opop)
            })
            .into()
    }
}

impl ModuleOp {
    /// Create a new [ModuleOp].
    /// The underlying [Operation] is not linked to a [BasicBlock].
    /// The returned module has a single [crate::region::Region] with a single (BasicBlock)[crate::basic_block::BasicBlock].
    pub fn new(ctx: &mut Context, name: Identifier) -> ModuleOp {
        let opop = Operation::new_op::<Self>(ctx, vec![], vec![], vec![], 1);
        opop.set_symbol_name(ctx, name);

        // Create an empty block.
        let region = opop.get_region(ctx);
        let block = BasicBlock::new(ctx, None, vec![]);
        block.insert_at_front(region, ctx);

        opop
    }
}

/// An operation with a name containing a single SSA control-flow-graph region.
#[pliron_op(
    name = "builtin.func",
    interfaces = [
        OneRegionInterface,
        SymbolOpInterface,
        IsolatedFromAboveInterface,
        NOpdsInterface<0>,
        NResultsInterface<0>
    ],
    attributes = (builtin_func_type : TypeAttr),
)]
pub struct FuncOp;

impl FuncOp {
    /// Create a new [FuncOp].
    /// The returned function has a single region with an empty `entry` block.
    pub fn new(ctx: &mut Context, name: Identifier, ty: TypedHandle<FunctionType>) -> Self {
        let ty_attr = TypeAttr::new(ty.into());
        let opop = Operation::new_op::<Self>(ctx, vec![], vec![], vec![], 1);
        let op = opop.get_operation();

        // Create an empty entry block.
        let arg_types = ty.deref(ctx).arg_types().clone();
        let region = op.deref_mut(ctx).get_region(0);
        let body = BasicBlock::new(ctx, Some(ident!("entry")), arg_types);
        body.insert_at_front(region, ctx);

        opop.set_symbol_name(ctx, name);
        opop.set_attr_builtin_func_type(ctx, ty_attr);

        opop
    }

    /// Get the function signature (type).
    pub fn get_type(&self, ctx: &Context) -> TypedHandle<FunctionType> {
        TypedHandle::from_handle(Typed::get_type(self, ctx), ctx)
            .expect("builtin.func must have a function type")
    }

    /// Get the entry block of this function.
    pub fn get_entry_block(&self, ctx: &Context) -> Ptr<BasicBlock> {
        self.get_region(ctx).deref(ctx).get_head().unwrap()
    }
}

impl Typed for FuncOp {
    fn get_type(&self, ctx: &Context) -> TypeHandle {
        let ty_attr = self.get_attr_builtin_func_type(ctx).unwrap();
        TypedAttrInterface::get_type(&*ty_attr, ctx)
    }
}

impl Printable for FuncOp {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        typed_symb_op_header(self).fmt(ctx, state, f)?;
        write!(f, " ")?;
        let mut attributes_to_print_separately =
            self.op.deref(ctx).attributes.clone_skip_outlined(ctx);
        attributes_to_print_separately
            .0
            .retain(|key, _| key != &ATTR_KEY_BUILTIN_FUNC_TYPE && key != &ATTR_KEY_SYM_NAME);
        if !attributes_to_print_separately.0.is_empty() {
            let _indent = state.indent();
            write!(f, "{}", indented_nl(state))?;
            attributes_to_print_separately.fmt(ctx, state, f)?;
        }
        region(self).fmt(ctx, state, f)?;
        Ok(())
    }
}

impl Parsable for FuncOp {
    type Arg = Vec<(Identifier, Location)>;
    type Parsed = OpObj;
    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        results: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        if !results.is_empty() {
            input_err!(
                state_stream.loc(),
                op_interfaces::NResultsVerifyErr(0, results.len())
            )?
        }

        let opop = Operation::new_op::<Self>(state_stream.state.ctx, vec![], vec![], vec![], 0);
        let op = opop.get_operation();

        let mut parser = (
            spaced(token('@').with(Identifier::parser(()))).skip(spaced(token(':'))),
            spaced(type_parser()),
            spaced(optional(AttributeDict::parser(()))),
            spaced(Region::parser(op)),
        );

        // Parse and build the function, providing name and type details.
        parser
            .parse_stream(state_stream)
            .map(|(fname, fty, attributes, _region)| -> OpObj {
                let ctx = &mut state_stream.state.ctx;
                op.deref_mut(ctx).attributes = attributes.unwrap_or_default();
                let ty_attr = TypeAttr::new(fty);
                opop.set_symbol_name(ctx, fname);
                opop.set_attr_builtin_func_type(ctx, ty_attr);
                OpObj::new(opop)
            })
            .into()
    }
}

#[derive(Error, Debug)]
#[error("function does not have function type")]
pub struct FuncOpTypeErr;

impl Verify for FuncOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let is_fn_type = self.get_attr_builtin_func_type(ctx).is_some_and(|ty_attr| {
            TypedAttrInterface::get_type(&*ty_attr, ctx)
                .deref(ctx)
                .is::<FunctionType>()
        });
        if !is_fn_type {
            return verify_err!(self.loc(ctx), FuncOpTypeErr);
        }
        let Some(entry) = self.get_region(ctx).deref(ctx).get_head() else {
            return Ok(());
        };
        verify_types_match(
            ctx,
            self.loc(ctx),
            entry.deref(ctx).arguments(),
            self.get_type(ctx).deref(ctx).arg_types(),
        )
    }
}

/// Terminates function execution and returns the function's results.
///
/// ### Operands
///
/// | operand | description |
/// |-----|-------|
/// | `operands` | return values, one per result of the parent function |
#[pliron_op(
    name = "builtin.return",
    format = "operands(CharSpace(`,`))",
    interfaces = [IsTerminatorInterface, NResultsInterface<0>],
)]
pub struct ReturnOp;

impl ReturnOp {
    /// Create a new [ReturnOp], returning `values`.
    pub fn new(ctx: &mut Context, values: Vec<Value>) -> Self {
        Operation::new_op::<Self>(ctx, vec![], values, vec![], 0)
    }
}

/// Build [FunctionType] from the argument and result types of `call`.
fn derived_callee_type(
    ctx: &Context,
    call: &dyn CallOpInterface,
) -> TypeInterfaceHandle<dyn FunctionTypeInterface> {
    let arg_types = call.args(ctx).iter().map(|arg| arg.get_type(ctx)).collect();
    let res_types = call.get_operation().deref(ctx).result_types().collect();
    FunctionType::get(ctx, arg_types, res_types).into()
}

impl Verify for ReturnOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let Some(func_op) = self
            .get_operation()
            .deref(ctx)
            .get_parent_op(ctx)
            .and_then(|parent| Operation::get_op::<FuncOp>(parent, ctx))
        else {
            // We're not inside a `builtin.func`, so can't verify the operand types.
            return Ok(());
        };
        let Some(func_ty) = func_op
            .get_attr_builtin_func_type(ctx)
            .map(|ty_attr| TypedAttrInterface::get_type(&*ty_attr, ctx))
            .and_then(|ty| TypedHandle::<FunctionType>::from_handle(ty, ctx).ok())
        else {
            // Let the function verifier report the missing `FunctionType` on it.
            return Ok(());
        };
        let res_types = func_ty.deref(ctx).res_types();
        verify_types_match(
            ctx,
            self.loc(ctx),
            self.get_operation().deref(ctx).operands(),
            res_types,
        )
    }
}

/// Direct call to a [FuncOp].
///
/// ### Attributes
///
/// | key | value |
/// |-----|-------|
/// | `builtin_call_callee` | [IdentifierAttr] naming the callee |
///
/// ### Operands
///
/// | operand | description |
/// |-----|-------|
/// | `args` | one value per argument of the callee |
///
/// ### Results
///
/// | result | description |
/// |-----|-------|
/// | `results` | one value per result of the callee |
#[pliron_op(
    name = "builtin.call",
    format = "`@` attr($builtin_call_callee, $IdentifierAttr) ` (` operands(CharSpace(`,`)) `) : ` typesig",
    attributes = (builtin_call_callee: IdentifierAttr),
)]
pub struct CallOp;

impl CallOp {
    /// Create a new [CallOp] to call `callee : callee_ty`, passing `args`.
    pub fn new(
        ctx: &mut Context,
        callee: Identifier,
        callee_ty: TypedHandle<FunctionType>,
        args: Vec<Value>,
    ) -> Self {
        let res_types = callee_ty.deref(ctx).res_types();
        let op = Operation::new_op::<Self>(ctx, res_types, args, vec![], 0);
        op.set_attr_builtin_call_callee(ctx, IdentifierAttr::new(callee));
        op
    }

    /// Get callee's symbol name.
    pub fn get_callee_name(&self, ctx: &Context) -> Identifier {
        self.get_attr_builtin_call_callee(ctx)
            .expect("builtin.call must have a callee")
            .clone()
            .into()
    }
}

#[op_interface_impl]
impl CallOpInterface for CallOp {
    fn callee(&self, ctx: &Context) -> CallOpCallable {
        CallOpCallable::Direct(self.get_callee_name(ctx))
    }

    fn arg_range(&self, ctx: &Context) -> Range<usize> {
        0..self.get_operation().deref(ctx).get_num_operands()
    }

    fn callee_type(&self, ctx: &Context) -> TypeInterfaceHandle<dyn FunctionTypeInterface> {
        derived_callee_type(ctx, self)
    }
}

#[op_interface_impl]
impl SymbolUserOpInterface for CallOp {
    fn verify_symbol_uses(
        &self,
        ctx: &Context,
        symbol_tables: &mut SymbolTableCollection,
    ) -> Result<()> {
        verify_func_symbol_use::<FuncOp>(
            ctx,
            self,
            symbol_tables,
            &self.get_callee_name(ctx),
            self.callee_type(ctx).to_handle(),
        )
    }

    fn used_symbols(&self, ctx: &Context) -> Vec<Identifier> {
        vec![self.get_callee_name(ctx)]
    }
}

#[derive(Error, Debug)]
#[error("builtin.call must have a callee symbol")]
pub struct CallOpNoCalleeErr;

impl Verify for CallOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        if self.get_attr_builtin_call_callee(ctx).is_none() {
            return verify_err!(self.loc(ctx), CallOpNoCalleeErr);
        }
        Ok(())
    }
}

/// Indirect call to a function value.
///
/// A function value typically comes from a [FuncConstantOp].
///
/// ### Operands
///
/// | operand | description |
/// |-----|-------|
/// | `callee` | value of [FunctionType] |
/// | `args` | one value per argument of the callee |
///
/// ### Results
///
/// | result | description |
/// |-----|-------|
/// | `results` | one value per result of the callee |
#[pliron_op(
    name = "builtin.indirect_call",
    interfaces = [AtLeastNOpdsInterface<1>],
)]
pub struct IndirectCallOp;

impl IndirectCallOp {
    /// Create a new [IndirectCallOp] to call `callee` with `args`.
    pub fn new(ctx: &mut Context, callee: Value, args: Vec<Value>) -> Self {
        let callee_ty = TypedHandle::<FunctionType>::from_handle(callee.get_type(ctx), ctx)
            .expect("callee of builtin.indirect_call must have a function type");
        Self::new_with_type(ctx, callee, callee_ty, args)
    }

    fn new_with_type(
        ctx: &mut Context,
        callee: Value,
        callee_ty: TypedHandle<FunctionType>,
        mut args: Vec<Value>,
    ) -> Self {
        let res_types = callee_ty.deref(ctx).res_types();
        args.insert(0, callee);
        Operation::new_op::<Self>(ctx, res_types, args, vec![], 0)
    }

    /// Get the called function value.
    pub fn get_callee(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(0)
    }
}

#[op_interface_impl]
impl CallOpInterface for IndirectCallOp {
    fn callee(&self, ctx: &Context) -> CallOpCallable {
        CallOpCallable::Indirect(self.get_callee(ctx))
    }

    fn arg_range(&self, ctx: &Context) -> Range<usize> {
        1..self.get_operation().deref(ctx).get_num_operands()
    }

    fn callee_type(&self, ctx: &Context) -> TypeInterfaceHandle<dyn FunctionTypeInterface> {
        derived_callee_type(ctx, self)
    }
}

impl Verify for IndirectCallOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let callee_ty = self.callee_type(ctx);
        verify_types_match(
            ctx,
            self.loc(ctx),
            [self.get_callee(ctx)],
            [callee_ty.to_handle()],
        )
    }
}

impl Printable for IndirectCallOp {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let results = self
            .get_operation()
            .deref(ctx)
            .results()
            .collect::<Vec<_>>();
        if !results.is_empty() {
            write!(
                f,
                "{} = ",
                list_with_sep(&results, ListSeparator::CharSpace(',')).print(ctx, state)
            )?;
        }
        let callee_sig = self.callee_type(ctx).deref(ctx).type_sig();
        write!(
            f,
            "{} {} ({}) : {}",
            self.get_opid(),
            self.get_callee(ctx).print(ctx, state),
            list_with_sep(&self.args(ctx), ListSeparator::CharSpace(',')).print(ctx, state),
            callee_sig.print(ctx, state)
        )
    }
}

impl Parsable for IndirectCallOp {
    type Arg = Vec<(Identifier, Location)>;
    type Parsed = OpObj;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        results: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let mut parser = (
            spaced(ssa_opd_parser()),
            spaced(delimited_list_parser('(', ')', ',', ssa_opd_parser())),
            spaced(token(':')).with(TypeSig::parser(())),
        )
            .then(move |(callee, args, callee_sig)| {
                let results = results.clone();
                combine::parser(move |state_stream: &mut StateStream<'a>| {
                    let ctx = &mut state_stream.state.ctx;
                    let callee_ty = FunctionType::get_from_sig(ctx, callee_sig.clone());
                    let op = IndirectCallOp::new_with_type(ctx, callee, callee_ty, args.clone());
                    process_parsed_ssa_defs(state_stream, &results, op.get_operation())?;
                    Ok(OpObj::new(op)).into_parse_result()
                })
            });
        parser.parse_stream(state_stream).into_result()
    }
}

/// Build an SSA value referring to a [FuncOp] using its symbol.
///
/// ### Attributes
///
/// | key | value |
/// |-----|-------|
/// | `builtin_func_constant_callee` | [IdentifierAttr] naming the function |
///
/// ### Results
///
/// | result | description |
/// |-----|-------|
/// | `result` | [FunctionType] of the named function |
#[pliron_op(
    name = "builtin.func_constant",
    format = "`@` attr($builtin_func_constant_callee, $IdentifierAttr) ` : ` type($0)",
    interfaces = [NOpdsInterface<0>, OneResultInterface],
    attributes = (builtin_func_constant_callee: IdentifierAttr),
)]
pub struct FuncConstantOp;

impl FuncConstantOp {
    /// Create a new [FuncConstantOp] referring to `callee : callee_ty`.
    pub fn new(
        ctx: &mut Context,
        callee: Identifier,
        callee_ty: TypedHandle<FunctionType>,
    ) -> Self {
        let op = Operation::new_op::<Self>(ctx, vec![callee_ty.into()], vec![], vec![], 0);
        op.set_attr_builtin_func_constant_callee(ctx, IdentifierAttr::new(callee));
        op
    }

    /// Get the function's symbol name.
    pub fn get_callee_name(&self, ctx: &Context) -> Identifier {
        self.get_attr_builtin_func_constant_callee(ctx)
            .expect("builtin.func_constant must have a callee")
            .clone()
            .into()
    }
}

#[derive(Error, Debug)]
pub enum FuncConstantOpVerifyErr {
    #[error("builtin.func_constant must have a callee symbol")]
    NoCallee,
    #[error("Result of builtin.func_constant must have a function type")]
    NotFunctionType,
}

impl Verify for FuncConstantOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        if self.get_attr_builtin_func_constant_callee(ctx).is_none() {
            return verify_err!(self.loc(ctx), FuncConstantOpVerifyErr::NoCallee);
        }
        if !self.result_type(ctx).deref(ctx).is::<FunctionType>() {
            return verify_err!(self.loc(ctx), FuncConstantOpVerifyErr::NotFunctionType);
        }
        Ok(())
    }
}

#[op_interface_impl]
impl SymbolUserOpInterface for FuncConstantOp {
    fn verify_symbol_uses(
        &self,
        ctx: &Context,
        symbol_tables: &mut SymbolTableCollection,
    ) -> Result<()> {
        verify_func_symbol_use::<FuncOp>(
            ctx,
            self,
            symbol_tables,
            &self.get_callee_name(ctx),
            self.result_type(ctx),
        )
    }

    fn used_symbols(&self, ctx: &Context) -> Vec<Identifier> {
        vec![self.get_callee_name(ctx)]
    }
}

/// An operation that always produces the same runtime value.
/// This constant value is specified as an attribute, which can be of any type.
///
/// ### Results:
///
/// | result | description |
/// |-----|-------|
/// | `result` | any type |
#[pliron_op(
    name = "builtin.constant",
    format = "`<` $builtin_constant_value `>` ` : ` type($0)",
    interfaces = [NOpdsInterface<0>, OneResultInterface],
    attributes = (builtin_constant_value),
)]
pub struct ConstantOp;

#[derive(Error, Debug)]
pub enum ConstantOpVerifyErr {
    #[error("ConstantOp does not have a value attribute")]
    MissingValue,
    #[error("ConstantOp's value attribute {0} does not implement TypedAttrInterface")]
    ValueNotTyped(String),
    #[error("The value attribute is of type {0}, but the constant is of type {1}")]
    ResultTypeMismatch(String, String),
}

impl Verify for ConstantOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);
        let result_type = self.result_type(ctx);

        let Some(value) = self.get_attr_builtin_constant_value(ctx) else {
            return verify_err!(loc, ConstantOpVerifyErr::MissingValue);
        };
        let value: &dyn Attribute = &**value;
        let Some(value) = attr_cast::<dyn TypedAttrInterface>(value) else {
            return verify_err!(
                loc,
                ConstantOpVerifyErr::ValueNotTyped(value.get_attr_id().to_string())
            );
        };

        if value.get_type(ctx) != result_type {
            return verify_err!(
                loc,
                ConstantOpVerifyErr::ResultTypeMismatch(
                    value.get_type(ctx).disp(ctx).to_string(),
                    result_type.disp(ctx).to_string()
                )
            );
        }
        Ok(())
    }
}

impl ConstantOp {
    /// Get the constant value that this Op defines.
    /// The [Ref] is a borrow of the containing [Operation] object.
    ///
    /// Use [pliron::dyn_clone::clone_box] to clone the value if required.
    pub fn get_value<'a>(&self, ctx: &'a Context) -> Ref<'a, dyn TypedAttrInterface> {
        Ref::map(
            self.get_attr_builtin_constant_value(ctx)
                .expect("ConstantOp must have a value attribute"),
            |attr| {
                attr_cast::<dyn TypedAttrInterface>(&**attr)
                    .expect("ConstantOp's value attribute must impl TypedAttrInterface")
            },
        )
    }

    /// Create a new [ConstantOp].
    pub fn new(ctx: &mut Context, value: Box<dyn TypedAttrInterface>) -> Self {
        let result_type = value.get_type(ctx);
        let op = Operation::new_op::<Self>(ctx, vec![result_type], vec![], vec![], 0);
        op.set_attr_builtin_constant_value(ctx, value);
        op
    }
}

/// A placeholder during parsing to refer to yet undefined operations.
/// MLIR [uses](https://github.com/llvm/llvm-project/blob/185b81e034ba60081023b6e59504dfffb560f3e3/mlir/lib/AsmParser/Parser.cpp#L1075)
/// [UnrealizedConversionCastOp](https://mlir.llvm.org/docs/Dialects/Builtin/#builtinunrealized_conversion_cast-unrealizedconversioncastop)
/// for this purpose.
#[pliron_op(
    name = "builtin.forward_ref",
    interfaces = [
        NOpdsInterface<0>,
        OneResultInterface,
    ],
)]
pub struct ForwardRefOp;

impl Printable for ForwardRefOp {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(
            f,
            "{} = {}",
            self.get_result(ctx).unique_name(ctx),
            self.get_opid().print(ctx, state),
        )
    }
}

#[derive(Error, Debug)]
#[error("{0} is a temporary Op during parsing. It must not exit in a well-formed program.")]
pub struct ForwardRefOpExistenceErr(String);

impl Verify for ForwardRefOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        verify_err!(
            self.loc(ctx),
            ForwardRefOpExistenceErr(self.get_result(ctx).unique_name(ctx).into())
        )
    }
}

impl Parsable for ForwardRefOp {
    type Arg = Vec<(Identifier, Location)>;
    type Parsed = OpObj;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _results: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        input_err!(
            state_stream.loc(),
            ForwardRefOpExistenceErr(
                ForwardRefOp::get_opid_static()
                    .disp(state_stream.state.ctx)
                    .to_string()
            )
        )?
    }
}

impl ForwardRefOp {
    /// Create a new [ForwardRefOp].
    pub fn new(ctx: &mut Context) -> Self {
        let ty = UnitType::get(ctx).into();
        Operation::new_op::<Self>(ctx, vec![ty], vec![], vec![], 0)
    }
}
