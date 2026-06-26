use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};
use thiserror::Error;

use crate::{
    attribute::{AttrObj, AttributeDict, attr_cast},
    basic_block::BasicBlock,
    builtin::{
        op_interfaces::{
            ATTR_KEY_SYM_NAME, NRegionsInterface, NResultsInterface, NoTerminatorInterface,
            RegionKind, RegionKindInterface,
        },
        ops::func_op_attr_names::ATTR_KEY_FUNC_TYPE,
        type_interfaces::FunctionTypeInterface,
    },
    combine::{Parser, optional, token},
    common_traits::{Named, Verify},
    context::{Context, Ptr},
    identifier::Identifier,
    indented_block, input_err,
    irfmt::{
        parsers::{spaced, type_parser},
        printers::op::{region, symb_op_header, typed_symb_op_header},
    },
    linked_list::ContainsLinkedList,
    location::{Located, Location},
    op::{Op, OpObj},
    operation::Operation,
    parsable::{Parsable, ParseResult, StateStream},
    printable::{self, Printable, indented_nl},
    region::Region,
    result::Result,
    r#type::{TypeHandle, Typed, TypedHandle},
    verify_err,
};
use pliron::derive::{op_interface_impl, pliron_op};

use super::{
    attr_interfaces::TypedAttrInterface,
    attributes::TypeAttr,
    op_interfaces::{
        self, IsolatedFromAboveInterface, NOpdsInterface, OneRegionInterface, OneResultInterface,
        SingleBlockRegionInterface, SymbolOpInterface, SymbolTableInterface,
    },
    types::{FunctionType, UnitType},
};

/// Represents a module, a top level container operation.
///
/// See MLIR's [builtin.module](https://mlir.llvm.org/docs/Dialects/Builtin/#builtinmodule-mlirmoduleop).
/// It contains a single [Graph](super::op_interfaces::RegionKind::Graph)
/// region containing a single block which can contain any operations and
/// does not have a terminator.
///
#[pliron_op(
    name = "builtin.module",
    interfaces = [
        NRegionsInterface<1>,
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
        let op = Operation::new(
            state_stream.state.ctx,
            Self::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        let mut parser =
            spaced(token('@').with(Identifier::parser(()))).and(spaced(Region::parser(op)));
        parser
            .parse_stream(state_stream)
            .map(|(name, _region)| -> OpObj {
                let op = ModuleOp { op };
                op.set_symbol_name(state_stream.state.ctx, name);
                OpObj::new(op)
            })
            .into()
    }
}

impl ModuleOp {
    /// Create a new [ModuleOp].
    /// The underlying [Operation] is not linked to a [BasicBlock].
    /// The returned module has a single [crate::region::Region] with a single (BasicBlock)[crate::basic_block::BasicBlock].
    pub fn new(ctx: &mut Context, name: Identifier) -> ModuleOp {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 1);
        let opop = ModuleOp { op };
        opop.set_symbol_name(ctx, name);

        // Create an empty block.
        let region = opop.get_region(ctx);
        let block = BasicBlock::new(ctx, None, vec![]);
        block.insert_at_front(region, ctx);

        opop
    }
}

/// An operation with a name containing a single SSA control-flow-graph region.
/// See MLIR's [func.func](https://mlir.llvm.org/docs/Dialects/Func/#funcfunc-mlirfuncfuncop).
#[pliron_op(
    name = "builtin.func",
    interfaces = [
        NRegionsInterface<1>,
        OneRegionInterface,
        SymbolOpInterface,
        IsolatedFromAboveInterface,
        NOpdsInterface<0>,
        NResultsInterface<0>
    ],
    attributes = (func_type : TypeAttr),
)]
pub struct FuncOp;

impl FuncOp {
    /// Create a new [FuncOp].
    /// The returned function has a single region with an empty `entry` block.
    pub fn new(ctx: &mut Context, name: Identifier, ty: TypedHandle<FunctionType>) -> Self {
        let ty_attr = TypeAttr::new(ty.into());
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 1);

        // Create an empty entry block.
        let arg_types = ty.deref(ctx).arg_types().clone();
        let region = op.deref_mut(ctx).get_region(0);
        let body = BasicBlock::new(ctx, Some("entry".try_into().unwrap()), arg_types);
        body.insert_at_front(region, ctx);

        let opop = FuncOp { op };
        opop.set_symbol_name(ctx, name);
        opop.set_attr_func_type(ctx, ty_attr);

        opop
    }

    /// Get the function signature (type).
    pub fn get_type(&self, ctx: &Context) -> TypeHandle {
        attr_cast::<dyn TypedAttrInterface>(&*self.get_attr_func_type(ctx).unwrap())
            .unwrap()
            .get_type(ctx)
    }

    /// Get the entry block of this function.
    pub fn get_entry_block(&self, ctx: &Context) -> Ptr<BasicBlock> {
        self.get_region(ctx).deref(ctx).get_head().unwrap()
    }
}

impl Typed for FuncOp {
    fn get_type(&self, ctx: &Context) -> TypeHandle {
        self.get_type(ctx)
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
            self.op.deref(ctx).attributes.clone_skip_outlined();
        attributes_to_print_separately
            .0
            .retain(|key, _| key != &*ATTR_KEY_FUNC_TYPE && key != &*ATTR_KEY_SYM_NAME);
        if !attributes_to_print_separately.0.is_empty() {
            indented_block!(state, {
                write!(f, "{}", indented_nl(state))?;
                attributes_to_print_separately.fmt(ctx, state, f)?;
            });
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

        let op = Operation::new(
            state_stream.state.ctx,
            Self::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );

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
                let opop = FuncOp { op };
                opop.set_symbol_name(ctx, fname);
                opop.set_attr_func_type(ctx, ty_attr);
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
        let op = &*self.get_operation().deref(ctx);
        let ty = self.get_type(ctx);
        if !(ty.deref(ctx).is::<FunctionType>()) {
            return verify_err!(op.loc(), FuncOpTypeErr);
        }
        Ok(())
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
    interfaces = [NOpdsInterface<0>, OneResultInterface, NResultsInterface<1>],
    attributes = (builtin_constant_value),
    verifier = "succ"
)]
pub struct ConstantOp;

impl ConstantOp {
    /// Get the constant value that this Op defines.
    pub fn get_value(&self, ctx: &Context) -> AttrObj {
        self.get_attr_builtin_constant_value(ctx).unwrap().clone()
    }

    /// Create a new [ConstantOp].
    pub fn new(ctx: &mut Context, value: AttrObj) -> Self {
        let result_type = attr_cast::<dyn TypedAttrInterface>(&*value)
            .expect("ConstantOp const value must provide TypedAttrInterface")
            .get_type(ctx);
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_type],
            vec![],
            vec![],
            0,
        );
        let op = ConstantOp { op };
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
        NResultsInterface<1>,
        NOpdsInterface<0>,
        OneResultInterface,
    ],
)]
pub struct ForwardRefOp;

impl Printable for ForwardRefOp {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(
            f,
            "{} = {}",
            self.get_result(ctx).unique_name(ctx),
            self.get_opid().disp(ctx),
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
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![ty],
            vec![],
            vec![],
            0,
        );
        ForwardRefOp { op }
    }
}
