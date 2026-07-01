//! [Op]s defined in the LLVM dialect

use pliron::{
    arg_err_noloc,
    attribute::{AttrObj, AttributeDict, attr_cast, attr_impls},
    basic_block::BasicBlock,
    builtin::{
        attr_interfaces::{FloatAttr, TypedAttrInterface},
        attributes::{BoolAttr, IdentifierAttr, IntegerAttr, StringAttr, TypeAttr},
        op_interfaces::{
            self, ATTR_KEY_SYM_NAME, AtLeastNOpdsInterface, AtLeastNResultsInterface,
            AtMostNOpdsInterface, AtMostNRegionsInterface, AtMostOneRegionInterface,
            BranchOpInterface, CallOpCallable, CallOpInterface, IsTerminatorInterface,
            IsolatedFromAboveInterface, NOpdsInterface, NResultsInterface, NSuccsInterface,
            OneOpdInterface, OneResultInterface, OneSuccInterface, OperandSegmentInterface,
            OptionalOpdInterface, SameOperandsAndResultType, SameOperandsType, SameResultsType,
            SingleBlockRegionInterface, SymbolOpInterface, SymbolUserOpInterface,
        },
        type_interfaces::{FloatTypeInterface, FunctionTypeInterface},
        types::{IntegerType, Signedness},
    },
    common_traits::{Named, Verify},
    context::{Context, Ptr},
    identifier::Identifier,
    indented_block, input_err,
    irfmt::{
        self,
        parsers::{
            attr_parser, block_opd_parser, delimited_list_parser, process_parsed_ssa_defs, spaced,
            ssa_opd_parser, type_parser,
        },
        printers::{iter_with_sep, list_with_sep, op::typed_symb_op_header},
    },
    linked_list::ContainsLinkedList,
    location::{Located, Location},
    op::{Op, OpObj},
    operation::Operation,
    parsable::{IntoParseResult, Parsable, ParseResult, StateStream},
    printable::{self, Printable, indented_nl},
    region::Region,
    result::{Error, ErrorKind, Result},
    symbol_table::SymbolTableCollection,
    r#type::{TypeHandle, TypedHandle, type_cast, type_impls},
    utils::vec_exns::VecExtns,
    value::Value,
    verify_err, verify_error,
};

use crate::{
    attributes::{
        AddressSpaceAttr, AlignmentAttr, AtomicOrderingAttr, AtomicRmwKindAttr, CaseValuesAttr,
        FCmpPredicateAttr, FastmathFlagsAttr, InsertExtractValueIndicesAttr, LinkageAttr,
        ShuffleVectorMaskAttr,
    },
    op_interfaces::{
        AlignableOpInterface, BinArithOp, CastOpInterface, CastOpWithNNegInterface, FastMathFlags,
        FloatBinArithOp, FloatBinArithOpWithFastMathFlags, IntBinArithOp,
        IntBinArithOpWithOverflowFlag, IsDeclaration, LlvmSymbolName, NNegFlag, PointerTypeResult,
    },
    ops::{
        func_op_attr_names::ATTR_KEY_LLVM_FUNC_TYPE,
        global_op_attr_names::{ATTR_KEY_GLOBAL_INITIALIZER, ATTR_KEY_LLVM_GLOBAL_TYPE},
    },
    types::{ArrayType, FuncType, StructType, VectorType},
};

#[cfg(feature = "llvm-sys")]
use crate::llvm_sys::core::{llvm_get_undef_mask_elem, llvm_lookup_intrinsic_id};

use pliron::combine::{
    self, between, optional,
    parser::{Parser, char::spaces},
    token,
};

use pliron::derive::{op_interface_impl, pliron_op};
use thiserror::Error;

use super::{
    attributes::{GepIndexAttr, GepIndicesAttr, ICmpPredicateAttr},
    types::PointerType,
};

/// Equivalent to LLVM's return opcode.
///
/// Operands:
///
/// | operand | description |
/// |-----|-------|
/// | `arg` | any type |
#[pliron_op(
    name = "llvm.return",
    format = "operands(CharSpace(`,`))",
    interfaces = [IsTerminatorInterface, NResultsInterface<0>, AtMostNOpdsInterface<1>, OptionalOpdInterface],
)]
pub struct ReturnOp;
impl ReturnOp {
    /// Create a new [ReturnOp]
    pub fn new(ctx: &mut Context, value: Option<Value>) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![],
            value.into_iter().collect(),
            vec![],
            0,
        );
        ReturnOp { op }
    }

    /// Get the returned value, if it exists.
    pub fn retval(&self, ctx: &Context) -> Option<Value> {
        self.get_operand_opt(ctx)
    }
}

#[derive(Error, Debug)]
enum ReturnOpVerifyErr {
    #[error("ReturnOp must have no operands in a void function")]
    VoidWithOperand,
    #[error("ReturnOp must have exactly one operand in a non-void function")]
    NonVoidArity,
    #[error("ReturnOp operand type does not match the function's result type")]
    ResultTypeMismatch,
}

impl Verify for ReturnOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        use pliron::r#type::Typed;
        // Signature coupling is only enforced when the return is inside a FuncOp.
        let Some(parent_op) = self.get_operation().deref(ctx).get_parent_op(ctx) else {
            return Ok(());
        };
        let Some(func_op) = Operation::get_op::<FuncOp>(parent_op, ctx) else {
            return Ok(());
        };
        let func_ty = func_op.get_type(ctx);
        let res_ty = func_ty.deref(ctx).result_type();
        let num_operands = self.get_operation().deref(ctx).get_num_operands();
        if res_ty.deref(ctx).is::<crate::types::VoidType>() {
            if num_operands != 0 {
                verify_err!(self.loc(ctx), ReturnOpVerifyErr::VoidWithOperand)?
            }
        } else if num_operands != 1 {
            verify_err!(self.loc(ctx), ReturnOpVerifyErr::NonVoidArity)?
        } else {
            let ret_ty = self.get_operation().deref(ctx).get_operand(0).get_type(ctx);
            if ret_ty != res_ty {
                verify_err!(self.loc(ctx), ReturnOpVerifyErr::ResultTypeMismatch)?
            }
        }
        Ok(())
    }
}

/// Equivalent to LLVM's unreachable opcode.
/// No operands or results.
#[pliron_op(
    name = "llvm.unreachable",
    format,
    interfaces = [IsTerminatorInterface, NOpdsInterface<0>, NResultsInterface<0>],
    verifier = "succ"
)]
pub struct UnreachableOp;

impl UnreachableOp {
    /// Create a new [UnreachableOp]
    pub fn new(ctx: &mut Context) -> Self {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        UnreachableOp { op }
    }
}

macro_rules! new_int_bin_op_with_format {
    (   $(#[$outer:meta])*
        $op_name:ident, $op_id:literal, $format:literal
    ) => {
        $(#[$outer])*
        /// ### Operands:
        ///
        /// | operand | description |
        /// |-----|-------|
        /// | `lhs` | Signless integer |
        /// | `rhs` | Signless integer |
        ///
        /// ### Result(s):
        ///
        /// | result | description |
        /// |-----|-------|
        /// | `res` | Signless integer |
        #[pliron_op(
            name = $op_id,
            format = $format,
            interfaces = [
                OneResultInterface, NResultsInterface<1>, SameOperandsType, SameResultsType,
                AtLeastNOpdsInterface<1>, AtLeastNResultsInterface<1>,
                SameOperandsAndResultType, BinArithOp, IntBinArithOp, NOpdsInterface<2>
            ],
            verifier = "succ"
        )]
        pub struct $op_name;
    }
}

macro_rules! new_int_bin_op {
    (   $(#[$outer:meta])*
        $op_name:ident, $op_id:literal
    ) => {
        new_int_bin_op_with_format!(
            $(#[$outer])*
            $op_name,
            $op_id,
            "$0 `, ` $1 ` : ` type($0)"
        );
    }
}

macro_rules! new_int_bin_op_with_overflow {
    (   $(#[$outer:meta])*
        $op_name:ident, $op_id:literal
    ) => {
        new_int_bin_op_with_format!(
            $(#[$outer])*
            /// ### Attributes:
            ///
            /// | key | value | via Interface |
            /// |-----|-------| --------------
            /// | [ATTR_KEY_INTEGER_OVERFLOW_FLAGS](super::op_interfaces::ATTR_KEY_INTEGER_OVERFLOW_FLAGS) | [IntegerOverflowFlagsAttr](super::attributes::IntegerOverflowFlagsAttr) | [IntBinArithOpWithOverflowFlag] |
            $op_name,
            $op_id,
            "$0 `, ` $1 ` <` attr($llvm_integer_overflow_flags, `super::attributes::IntegerOverflowFlagsAttr`) `>` `: ` type($0)"
        );
        #[pliron::derive::op_interface_impl]
        impl IntBinArithOpWithOverflowFlag for $op_name {}
    }
}

new_int_bin_op_with_overflow!(
    /// Equivalent to LLVM's Add opcode.
    AddOp,
    "llvm.add"
);

new_int_bin_op_with_overflow!(
    /// Equivalent to LLVM's Sub opcode.
    SubOp,
    "llvm.sub"
);

new_int_bin_op_with_overflow!(
    /// Equivalent to LLVM's Mul opcode.
    MulOp,
    "llvm.mul"
);

new_int_bin_op_with_overflow!(
    /// Equivalent to LLVM's Shl opcode.
    ShlOp,
    "llvm.shl"
);

new_int_bin_op!(
    /// Equivalent to LLVM's UDiv opcode.
    UDivOp,
    "llvm.udiv"
);

new_int_bin_op!(
    /// Equivalent to LLVM's SDiv opcode.
    SDivOp,
    "llvm.sdiv"
);

new_int_bin_op!(
    /// Equivalent to LLVM's URem opcode.
    URemOp,
    "llvm.urem"
);

new_int_bin_op!(
    /// Equivalent to LLVM's SRem opcode.
    SRemOp,
    "llvm.srem"
);

new_int_bin_op!(
    /// Equivalent to LLVM's And opcode.
    AndOp,
    "llvm.and"
);

new_int_bin_op!(
    /// Equivalent to LLVM's Or opcode.
    OrOp,
    "llvm.or"
);

new_int_bin_op!(
    /// Equivalent to LLVM's Xor opcode.
    XorOp,
    "llvm.xor"
);

new_int_bin_op!(
    /// Equivalent to LLVM's LShr opcode.
    LShrOp,
    "llvm.lshr"
);

new_int_bin_op!(
    /// Equivalent to LLVM's AShr opcode.
    AShrOp,
    "llvm.ashr"
);

#[derive(Error, Debug)]
pub enum ICmpOpVerifyErr {
    #[error("Result must be (possibly vector of) 1-bit integer (bool)")]
    ResultNotBool,
    #[error("Operand must be (possibly vector of) integer or pointer types")]
    IncorrectOperandsType,
    #[error("Missing or incorrect predicate attribute")]
    PredAttrErr,
    #[error("Vector operand and result types must have the same number of elements")]
    MismatchedVectorNumElements,
}

/// Equivalent to LLVM's ICmp opcode.
/// ### Operand(s):
/// | operand | description |
/// |-----|-------|
/// | `lhs` | Signless integer or pointer |
/// | `rhs` | Signless integer or pointer |
///
/// ### Result(s):
///
/// | result | description |
/// |-----|-------|
/// | `res` | 1-bit signless integer |
#[pliron_op(
    name = "llvm.icmp",
    format = "$0 ` <` attr($icmp_predicate, $ICmpPredicateAttr) `> ` $1 ` : ` type($0)",
    interfaces = [SameOperandsType, AtLeastNOpdsInterface<1>, OneResultInterface, NResultsInterface<1>],
    attributes = (icmp_predicate: ICmpPredicateAttr)
)]
pub struct ICmpOp;

impl ICmpOp {
    /// Create a new [ICmpOp]
    pub fn new(ctx: &mut Context, pred: ICmpPredicateAttr, lhs: Value, rhs: Value) -> Self {
        use pliron::r#type::Typed;

        // Determine the result type.
        let bool_ty = IntegerType::get(ctx, 1, Signedness::Signless);
        let opd_type = lhs.get_type(ctx);
        let vec_details = opd_type
            .deref(ctx)
            .downcast_ref::<VectorType>()
            .map(|vec_ty| (vec_ty.num_elements(), vec_ty.kind()));
        let res_ty = if let Some((num_elements, kind)) = vec_details {
            VectorType::get(ctx, bool_ty.into(), num_elements, kind).into()
        } else {
            bool_ty.into()
        };

        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![res_ty],
            vec![lhs, rhs],
            vec![],
            0,
        );
        let op = ICmpOp { op };
        op.set_attr_icmp_predicate(ctx, pred);
        op
    }

    /// Get the predicate
    pub fn predicate(&self, ctx: &Context) -> ICmpPredicateAttr {
        self.get_attr_icmp_predicate(ctx)
            .expect("ICmpOp missing or incorrect predicate attribute type")
            .clone()
    }
}

impl Verify for ICmpOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);

        if self.get_attr_icmp_predicate(ctx).is_none() {
            verify_err!(loc.clone(), ICmpOpVerifyErr::PredAttrErr)?
        }

        let mut res_ty = self.result_type(ctx);
        let mut vec_num_elements = None;
        if let Some(vec_ty) = res_ty.deref(ctx).downcast_ref::<VectorType>() {
            res_ty = vec_ty.elem_type();
            vec_num_elements = Some(vec_ty.num_elements());
        }
        let res_ty = res_ty.deref(ctx);
        let Some(res_ty) = res_ty.downcast_ref::<IntegerType>() else {
            return verify_err!(loc, ICmpOpVerifyErr::ResultNotBool);
        };
        if res_ty.width() != 1 {
            return verify_err!(loc, ICmpOpVerifyErr::ResultNotBool);
        }

        let mut opd_ty = self.operand_type(ctx);
        if let Some(vec_ty) = opd_ty.deref(ctx).downcast_ref::<VectorType>() {
            opd_ty = vec_ty.elem_type();
            // Ensure that the number of elements matches the result type's number of elements.
            if vec_num_elements.is_none_or(|num_elements| vec_ty.num_elements() != num_elements) {
                return verify_err!(loc, ICmpOpVerifyErr::MismatchedVectorNumElements);
            }
        } else if vec_num_elements.is_some() {
            return verify_err!(loc, ICmpOpVerifyErr::MismatchedVectorNumElements);
        }
        let opd_ty = opd_ty.deref(ctx);
        if !(opd_ty.is::<IntegerType>() || opd_ty.is::<PointerType>()) {
            return verify_err!(loc, ICmpOpVerifyErr::IncorrectOperandsType);
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum AllocaOpVerifyErr {
    #[error("Operand must be a signless integer")]
    OperandType,
    #[error("Missing or incorrect type of attribute for element type")]
    ElemTypeAttr,
}

/// Equivalent to LLVM's Alloca opcode.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `array_size` | Signless integer |
///
/// ### Result(s):
///
/// | result | description |
/// |-----|-------|
/// | `res` | [PointerType] |
#[pliron_op(
    name = "llvm.alloca",
    format = "`[` attr($alloca_element_type, $TypeAttr) ` x ` $0 `]` ` ` \
    opt_attr($llvm_alignment, $AlignmentAttr, label($align), delimiters(`[`, `]`)) \
    ` : ` type($0)",
    interfaces = [
        OneResultInterface,
        OneOpdInterface,
        NResultsInterface<1>,
        NOpdsInterface<1>,
        AlignableOpInterface,
    ],
    operands = (array_size: IntegerType),
    results = (_: PointerType),
    attributes = (alloca_element_type: TypeAttr)
)]
pub struct AllocaOp;
impl Verify for AllocaOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);
        // Ensure correctness of element type.
        if self.get_attr_alloca_element_type(ctx).is_none() {
            verify_err!(loc, AllocaOpVerifyErr::ElemTypeAttr)?
        }
        Ok(())
    }
}

#[op_interface_impl]
impl PointerTypeResult for AllocaOp {
    fn result_pointee_type(&self, ctx: &Context) -> TypeHandle {
        self.get_attr_alloca_element_type(ctx)
            .expect("AllocaOp missing or incorrect type for elem_type attribute")
            .get_type(ctx)
    }
}

impl AllocaOp {
    /// Create a new [AllocaOp]
    pub fn new(ctx: &mut Context, elem_type: TypeHandle, size: Value) -> Self {
        // `alloca` yields a pointer in the default (stack) address space.
        let ptr_ty = PointerType::get(ctx, 0).into();
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![ptr_ty],
            vec![size],
            vec![],
            0,
        );
        let op = AllocaOp { op };
        op.set_attr_alloca_element_type(ctx, TypeAttr::new(elem_type));
        op
    }
}

/// Equivalent to LLVM's Bitcast opcode.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | non-aggregate LLVM type |
///
/// ### Result(s):
///
/// | result | description |
/// |-----|-------|
/// | `res` | non-aggregate LLVM type |
#[pliron_op(
    name = "llvm.bitcast",
    format = "$0 ` to ` type($0)",
    interfaces = [
        OneResultInterface,
        OneOpdInterface,
        NResultsInterface<1>,
        NOpdsInterface<1>,
        CastOpInterface
    ],
    verifier = "succ"
)]
pub struct BitcastOp;

#[derive(Error, Debug)]
pub enum IntToPtrOpErr {
    #[error("Operand must be a signless integer")]
    OperandTypeErr,
    #[error("Result must be a pointer type")]
    ResultTypeErr,
}

/// Equivalent to LLVM's IntToPtr opcode.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | Signless integer |
////
/// ### Result(s):
///
/// | result | description |
/// |-----|-------|
/// | `res` | [PointerType] |
///
#[pliron_op(
    name = "llvm.inttoptr",
    format = "$0 ` to ` type($0)",
    interfaces = [
        OneResultInterface,
        OneOpdInterface,
        NResultsInterface<1>,
        NOpdsInterface<1>,
        CastOpInterface,
     ],
      operands = (arg: IntegerType),
    results = (_: PointerType),
     verifier = "succ"
)]
pub struct IntToPtrOp;

#[derive(Error, Debug)]
pub enum PtrToIntOpErr {
    #[error("Operand must be a pointer type")]
    OperandTypeErr,
    #[error("Result must be a signless integer type")]
    ResultTypeErr,
}

/// Equivalent to LLVM's PtrToInt opcode.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | [PointerType] |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | Signless integer |
#[pliron_op(
    name = "llvm.ptrtoint",
    format = "$0 ` to ` type($0)",
    interfaces = [
        OneResultInterface,
        OneOpdInterface,
        NResultsInterface<1>,
        NOpdsInterface<1>,
        CastOpInterface,
    ],
    operands = (arg: PointerType),
    results = (_: IntegerType),
    verifier = "succ"
)]
pub struct PtrToIntOp;

/// Equivalent to LLVM's AddrSpaceCast opcode: casts a pointer to a pointer in a
/// different address space.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | [PointerType] |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | [PointerType] |
#[pliron_op(
    name = "llvm.addrspacecast",
    format = "$0 ` to ` type($0)",
    interfaces = [
        OneResultInterface,
        OneOpdInterface,
        NResultsInterface<1>,
        NOpdsInterface<1>,
        CastOpInterface,
    ],
    operands = (arg: PointerType),
    results = (_: PointerType),
    verifier = "succ"
)]
pub struct AddrSpaceCastOp;

/// Equivalent to LLVM's Unconditional Branch.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `dest_opds` | Any number of operands with any LLVM type |
///
/// ### Successors:
///
/// | Successor | description |
/// |-----|-------|
/// | `dest` | Any successor |
#[pliron_op(
    name = "llvm.br",
    format = "succ($0) `(` operands(CharSpace(`,`)) `)`",
    interfaces = [
        IsTerminatorInterface,
        NResultsInterface<0>,
        NSuccsInterface<1>,
        OneSuccInterface
    ],
    verifier = "succ"
)]
pub struct BrOp;

#[op_interface_impl]
impl BranchOpInterface for BrOp {
    fn successor_operands(&self, ctx: &Context, succ_idx: usize) -> Vec<Value> {
        assert!(succ_idx == 0, "BrOp has exactly one successor");
        self.get_operation().deref(ctx).operands().collect()
    }

    fn add_successor_operand(&self, ctx: &mut Context, succ_idx: usize, operand: Value) -> usize {
        assert!(succ_idx == 0, "BrOp has exactly one successor");
        Operation::push_operand(self.get_operation(), ctx, operand)
    }

    fn remove_successor_operand(
        &self,
        ctx: &mut Context,
        succ_idx: usize,
        opd_idx: usize,
    ) -> Value {
        assert!(succ_idx == 0, "BrOp has exactly one successor");
        Operation::remove_operand(self.get_operation(), ctx, opd_idx)
    }
}

impl BrOp {
    /// Create anew [BrOp].
    pub fn new(ctx: &mut Context, dest: Ptr<BasicBlock>, dest_opds: Vec<Value>) -> Self {
        BrOp {
            op: Operation::new(
                ctx,
                Self::get_concrete_op_info(),
                vec![],
                dest_opds,
                vec![dest],
                0,
            ),
        }
    }
}

/// Equivalent to LLVM's Conditional Branch.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `condition` | 1-bit signless integer |
/// | `true_dest_opds` | Any number of operands with any LLVM type |
/// | `false_dest_opds` | Any number of operands with any LLVM type |
///
/// ### Successors:
///
/// | Successor | description |
/// |-----|-------|
/// | `true_dest` | Any successor |
/// | `false_dest` | Any successor |
#[pliron_op(
    name = "llvm.cond_br",
    interfaces = [IsTerminatorInterface, NResultsInterface<0>, NSuccsInterface<2>],
    operands = (condition, true_dest_opds, false_dest_opds),
)]
pub struct CondBrOp;
impl CondBrOp {
    /// Create a new [CondBrOp].
    pub fn new(
        ctx: &mut Context,
        condition: Value,
        true_dest: Ptr<BasicBlock>,
        true_dest_opds: Vec<Value>,
        false_dest: Ptr<BasicBlock>,
        false_dest_opds: Vec<Value>,
    ) -> Self {
        let (operands, segment_sizes) =
            Self::compute_segment_sizes(vec![vec![condition], true_dest_opds, false_dest_opds]);

        let op = CondBrOp {
            op: Operation::new(
                ctx,
                Self::get_concrete_op_info(),
                vec![],
                operands,
                vec![true_dest, false_dest],
                0,
            ),
        };

        // Set the operand segment sizes attribute.
        op.set_operand_segment_sizes(ctx, segment_sizes);
        op
    }
}

#[derive(Error, Debug)]
enum CondBrOpVerifyErr {
    #[error("Condition operand must be a 1-bit signless integer (i1) or vector of i1")]
    IncorrectConditionType,
}

impl Verify for CondBrOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        use pliron::r#type::Typed;
        // Ensure that the condition is a 1-bit signless integer
        let condition_ty = self.get_operand_condition(ctx).get_type(ctx);
        let condition_ty = condition_ty.deref(ctx);
        let condition_int_ty = condition_ty.downcast_ref::<IntegerType>().ok_or_else(|| {
            verify_error!(self.loc(ctx), CondBrOpVerifyErr::IncorrectConditionType)
        })?;
        if condition_int_ty.width() != 1 || condition_int_ty.signedness() != Signedness::Signless {
            verify_err!(self.loc(ctx), CondBrOpVerifyErr::IncorrectConditionType)?
        }
        Ok(())
    }
}

#[op_interface_impl]
impl OperandSegmentInterface for CondBrOp {}

impl Printable for CondBrOp {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &pliron::printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let op = self.get_operation().deref(ctx);
        let condition = op.get_operand(0);
        let true_dest_opds = self.successor_operands(ctx, 0);
        let false_dest_opds = self.successor_operands(ctx, 1);
        let res = write!(
            f,
            "{} if {} ^{}({}) else ^{}({})",
            Self::get_opid_static(),
            condition.disp(ctx),
            op.get_successor(0).deref(ctx).unique_name(ctx),
            iter_with_sep(
                true_dest_opds.iter(),
                pliron::printable::ListSeparator::CharSpace(',')
            )
            .disp(ctx),
            op.get_successor(1).deref(ctx).unique_name(ctx),
            iter_with_sep(
                false_dest_opds.iter(),
                pliron::printable::ListSeparator::CharSpace(',')
            )
            .disp(ctx),
        );
        res
    }
}

impl Parsable for CondBrOp {
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

        // Parse the condition operand.
        let r#if = irfmt::parsers::spaced::<StateStream, _>(combine::parser::char::string("if"));

        let condition = ssa_opd_parser();

        let true_operands = delimited_list_parser('(', ')', ',', ssa_opd_parser());

        let r_else =
            irfmt::parsers::spaced::<StateStream, _>(combine::parser::char::string("else"));

        let false_operands = delimited_list_parser('(', ')', ',', ssa_opd_parser());

        let final_parser = r#if
            .with(spaced(condition))
            .and(spaced(block_opd_parser()))
            .and(true_operands)
            .and(spaced(r_else).with(spaced(block_opd_parser()).and(false_operands)));

        final_parser
            .then(
                move |(((condition, true_dest), true_dest_opds), (false_dest, false_dest_opds))| {
                    let results = results.clone();
                    combine::parser(move |parsable_state: &mut StateStream<'a>| {
                        let ctx = &mut parsable_state.state.ctx;
                        let op = CondBrOp::new(
                            ctx,
                            condition,
                            true_dest,
                            true_dest_opds.clone(),
                            false_dest,
                            false_dest_opds.clone(),
                        );

                        process_parsed_ssa_defs(parsable_state, &results, op.get_operation())?;
                        Ok(OpObj::new(op)).into_parse_result()
                    })
                },
            )
            .parse_stream(state_stream)
            .into()
    }
}

#[op_interface_impl]
impl BranchOpInterface for CondBrOp {
    fn successor_operands(&self, ctx: &Context, succ_idx: usize) -> Vec<Value> {
        assert!(
            succ_idx == 0 || succ_idx == 1,
            "CondBrOp has exactly two successors"
        );

        // Skip the first segment, which is the condition.
        self.get_segment(ctx, succ_idx + 1)
    }

    fn add_successor_operand(&self, ctx: &mut Context, succ_idx: usize, operand: Value) -> usize {
        // The successor operands start at segment 1, since segment 0 is the condition operand.
        self.push_to_segment(ctx, succ_idx + 1, operand)
    }

    fn remove_successor_operand(
        &self,
        ctx: &mut Context,
        succ_idx: usize,
        opd_idx: usize,
    ) -> Value {
        // The successor operands start at segment 1, since segment 0 is the condition operand.
        self.remove_from_segment(ctx, succ_idx + 1, opd_idx)
    }
}

/// Equivalent to LLVM's Switch opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `condition` | integer (width matches the case values) |
/// | `default_dest_opds` | variadic of any type |
/// | `case_dest_opds` | variadic of any type |
///
/// ### Successors:
/// | Successor | description |
/// |-----|-------|
/// | `default_dest` | any successor |
/// | `case_dests` | any successor(s) |
#[pliron_op(
    name = "llvm.switch",
    interfaces = [IsTerminatorInterface, NResultsInterface<0>],
    operands = (condition, default_dest_opds, case_dest_opds),
    attributes = (switch_case_values: CaseValuesAttr)
)]
pub struct SwitchOp;

/// One case of a switch statement.
#[derive(Clone)]
pub struct SwitchCase {
    /// The value being matched against.
    pub value: IntegerAttr,
    /// The destination block to jump to if this case is taken.
    pub dest: Ptr<BasicBlock>,
    /// The operands to pass to the destination block.
    pub dest_opds: Vec<Value>,
}

impl Printable for SwitchCase {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &pliron::printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(
            f,
            "{{ {}: ^{}({}) }}",
            self.value.disp(ctx),
            self.dest.deref(ctx).unique_name(ctx),
            list_with_sep(
                &self.dest_opds,
                pliron::printable::ListSeparator::CharSpace(',')
            )
            .disp(ctx)
        )
    }
}

impl Parsable for SwitchCase {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let mut parser = between(
            token('{'),
            token('}'),
            (
                spaced(IntegerAttr::parser(())),
                spaced(token(':')),
                spaced(block_opd_parser()),
                delimited_list_parser('(', ')', ',', ssa_opd_parser()),
                spaces(),
            ),
        );

        let ((value, _colon, dest, dest_opds, _spaces), _) =
            parser.parse_stream(state_stream).into_result()?;

        Ok(SwitchCase {
            value,
            dest,
            dest_opds,
        })
        .into_parse_result()
    }
}

impl Printable for SwitchOp {
    fn fmt(
        &self,
        ctx: &Context,
        state: &pliron::printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let op = self.get_operation().deref(ctx);
        let condition = op.get_operand(0);

        let default_successor = op
            .successors()
            .next()
            .expect("SwitchOp must have at least one successor");
        let num_total_successors = op.get_num_successors();

        write!(
            f,
            "{} {}, ^{}({})",
            Self::get_opid_static(),
            condition.disp(ctx),
            default_successor.unique_name(ctx).disp(ctx),
            iter_with_sep(
                self.successor_operands(ctx, 0).iter(),
                pliron::printable::ListSeparator::CharSpace(',')
            )
            .disp(ctx),
        )?;

        if num_total_successors < 2 {
            writeln!(f, "[]")?;
            return Ok(());
        }

        let cases = self.cases(ctx);

        write!(f, "{}[", indented_nl(state))?;
        indented_block!(state, {
            write!(f, "{}", indented_nl(state))?;
            list_with_sep(&cases, pliron::printable::ListSeparator::CharNewline(','))
                .fmt(ctx, state, f)?;
        });
        write!(f, "{}]", indented_nl(state))?;

        Ok(())
    }
}

impl Parsable for SwitchOp {
    type Arg = Vec<(Identifier, Location)>;
    type Parsed = OpObj;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        if !arg.is_empty() {
            input_err!(
                state_stream.loc(),
                op_interfaces::NResultsVerifyErr(0, arg.len())
            )?
        }

        // Parse the condition operand.
        let condition = ssa_opd_parser().skip(spaced(token(',')));
        let default_successor = block_opd_parser();
        let default_operands = delimited_list_parser('(', ')', ',', ssa_opd_parser());
        let cases = delimited_list_parser('[', ']', ',', SwitchCase::parser(()));

        let final_parser = spaced(condition)
            .and(default_successor)
            .skip(spaces())
            .and(default_operands)
            .skip(spaces())
            .and(cases);

        final_parser
            .then(
                move |(((condition, default_dest), default_dest_opds), cases)| {
                    let results = arg.clone();
                    combine::parser(move |parsable_state: &mut StateStream<'a>| {
                        let ctx = &mut parsable_state.state.ctx;
                        let op = SwitchOp::new(
                            ctx,
                            condition,
                            default_dest,
                            default_dest_opds.clone(),
                            cases.clone(),
                        );

                        process_parsed_ssa_defs(parsable_state, &results, op.get_operation())?;
                        Ok(OpObj::new(op)).into_parse_result()
                    })
                },
            )
            .parse_stream(state_stream)
            .into()
    }
}

impl SwitchOp {
    /// Create a new [SwitchOp].
    pub fn new(
        ctx: &mut Context,
        condition: Value,
        default_dest: Ptr<BasicBlock>,
        default_dest_opds: Vec<Value>,
        cases: Vec<SwitchCase>,
    ) -> Self {
        let case_values: Vec<IntegerAttr> = cases.iter().map(|case| case.value.clone()).collect();

        let case_operands = cases
            .iter()
            .map(|case| case.dest_opds.clone())
            .collect::<Vec<_>>();

        let mut operand_segments = vec![vec![condition], default_dest_opds];
        operand_segments.extend(case_operands);
        let (operands, segment_sizes) = Self::compute_segment_sizes(operand_segments);

        let case_dests = cases.iter().map(|case| case.dest);
        let successors = vec![default_dest].into_iter().chain(case_dests).collect();
        let op = SwitchOp {
            op: Operation::new(
                ctx,
                Self::get_concrete_op_info(),
                vec![],
                operands,
                successors,
                0,
            ),
        };

        // Set the operand segment sizes attribute.
        op.set_operand_segment_sizes(ctx, segment_sizes);
        // Set the case values
        op.set_attr_switch_case_values(ctx, CaseValuesAttr(case_values));
        op
    }

    /// Get the cases of this switch operation.
    /// (The default case cannot be / isn't included here).
    pub fn cases(&self, ctx: &Context) -> Vec<SwitchCase> {
        let case_values = &*self
            .get_attr_switch_case_values(ctx)
            .expect("SwitchOp missing or incorrect case values attribute");

        let op = self.get_operation().deref(ctx);
        // Skip the first one, which is the default successor.
        let successors = op.successors().skip(1);

        successors
            .zip(case_values.0.iter())
            .enumerate()
            .map(|(i, (dest, value))| {
                // i+1 here because the first successor is the default destination.
                let dest_opds = self.successor_operands(ctx, i + 1);
                SwitchCase {
                    value: value.clone(),
                    dest,
                    dest_opds,
                }
            })
            .collect()
    }

    /// Get the default destination of this switch operation.
    pub fn default_dest(&self, ctx: &Context) -> Ptr<BasicBlock> {
        self.get_operation().deref(ctx).get_successor(0)
    }

    /// Get the operands to pass to the default destination.
    pub fn default_dest_operands(&self, ctx: &Context) -> Vec<Value> {
        self.successor_operands(ctx, 0)
    }
}

#[op_interface_impl]
impl BranchOpInterface for SwitchOp {
    fn successor_operands(&self, ctx: &Context, succ_idx: usize) -> Vec<Value> {
        // Skip the first segment, which is the condition.
        self.get_segment(ctx, succ_idx + 1)
    }

    fn add_successor_operand(&self, ctx: &mut Context, succ_idx: usize, operand: Value) -> usize {
        // The successor operands start at segment 1, since segment 0 is the condition operand.
        self.push_to_segment(ctx, succ_idx + 1, operand)
    }

    fn remove_successor_operand(
        &self,
        ctx: &mut Context,
        succ_idx: usize,
        opd_idx: usize,
    ) -> Value {
        // The successor operands start at segment 1, since segment 0 is the condition operand.
        self.remove_from_segment(ctx, succ_idx + 1, opd_idx)
    }
}

#[op_interface_impl]
impl OperandSegmentInterface for SwitchOp {}

#[derive(Error, Debug)]
pub enum SwitchOpVerifyErr {
    #[error("SwitchOp has no or incorrect case values attribute")]
    CaseValuesAttrErr,
    #[error("SwitchOp has no or incorrect default destination")]
    DefaultDestErr,
    #[error("SwitchOp has no condition operand or is not an integer")]
    ConditionErr,
}

impl Verify for SwitchOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);

        let Some(case_values) = self.get_attr_switch_case_values(ctx) else {
            verify_err!(loc.clone(), SwitchOpVerifyErr::CaseValuesAttrErr)?
        };

        let op = &*self.get_operation().deref(ctx);

        if op.get_num_successors() < 1 {
            verify_err!(loc.clone(), SwitchOpVerifyErr::DefaultDestErr)?;
        }

        if op.get_num_operands() < 1 {
            verify_err!(loc.clone(), SwitchOpVerifyErr::ConditionErr)?;
        }

        let condition_ty = pliron::r#type::Typed::get_type(&op.get_operand(0), ctx);
        let condition_ty = TypedHandle::<IntegerType>::from_handle(condition_ty, ctx)?;

        if let Some(case_value) = case_values.0.first() {
            // Ensure that the case value type matches the condition type.
            if case_value.get_type() != condition_ty {
                verify_err!(loc, SwitchOpVerifyErr::ConditionErr)?;
            }
        }

        Ok(())
    }
}

/// A way to express whether a GEP index is a constant or an SSA value
#[derive(Clone)]
pub enum GepIndex {
    Constant(u32),
    Value(Value),
}

impl Printable for GepIndex {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &pliron::printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            GepIndex::Constant(c) => write!(f, "{c}"),
            GepIndex::Value(v) => write!(f, "{}", v.disp(ctx)),
        }
    }
}

#[derive(Error, Debug)]
pub enum GetElementPtrOpErr {
    #[error("GetElementPtrOp has no or incorrect indices attribute")]
    IndicesAttrErr,
    #[error("The indices on this GEP are invalid for its source element type")]
    IndicesErr,
}

/// Equivalent to LLVM's GetElementPtr.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `base` | LLVM pointer type |
/// | `dynamicIndices` | Any number of signless integers |
///
/// ### Result(s):
///
/// | result | description |
/// |-----|-------|
/// | `res` | LLVM pointer type |
#[pliron_op(
    name = "llvm.gep",
    format = "`<` attr($gep_src_elem_type, $TypeAttr) `>` ` (` operands(CharSpace(`,`)) `)` attr($gep_indices, $GepIndicesAttr) ` : ` type($0)",
    interfaces = [OneResultInterface, NResultsInterface<1>],
    operands = (src_ptr, dynamic_indices),
    results = (_: PointerType),
    attributes = (gep_src_elem_type: TypeAttr, gep_indices: GepIndicesAttr)
)]
pub struct GetElementPtrOp;

#[op_interface_impl]
impl PointerTypeResult for GetElementPtrOp {
    fn result_pointee_type(&self, ctx: &Context) -> TypeHandle {
        Self::indexed_type(ctx, self.src_elem_type(ctx), &self.indices(ctx))
            .expect("Invalid indices for GEP")
    }
}

impl Verify for GetElementPtrOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);
        // Ensure that we have the indices as an attribute.
        if self.get_attr_gep_indices(ctx).is_none() {
            verify_err!(loc, GetElementPtrOpErr::IndicesAttrErr)?
        }

        if let Err(e @ Error { .. }) =
            Self::indexed_type(ctx, self.src_elem_type(ctx), &self.indices(ctx))
        {
            return Err(Error {
                kind: ErrorKind::VerificationFailed,
                // We reset the error origin to be from here
                backtrace: pliron::std_deps::backtrace::Backtrace::capture(),
                ..e
            });
        }

        Ok(())
    }
}

impl GetElementPtrOp {
    /// Create a new [GetElementPtrOp]
    pub fn new(
        ctx: &mut Context,
        base: Value,
        indices: Vec<GepIndex>,
        src_elem_type: TypeHandle,
    ) -> Self {
        use pliron::r#type::Typed;

        // A GEP result inherits the address space of its base pointer.
        let addr_space = {
            let base_ty = base.get_type(ctx);
            base_ty
                .deref(ctx)
                .downcast_ref::<PointerType>()
                .map_or(0, PointerType::address_space)
        };
        let result_type = PointerType::get(ctx, addr_space).into();
        let mut attr: Vec<GepIndexAttr> = Vec::new();
        let mut opds: Vec<Value> = vec![base];
        for idx in indices {
            match idx {
                GepIndex::Constant(c) => {
                    attr.push(GepIndexAttr::Constant(c));
                }
                GepIndex::Value(v) => {
                    attr.push(GepIndexAttr::OperandIdx(opds.push_back(v)));
                }
            }
        }
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_type],
            opds,
            vec![],
            0,
        );
        let src_elem_type = TypeAttr::new(src_elem_type);
        let op = GetElementPtrOp { op };

        op.set_attr_gep_indices(ctx, GepIndicesAttr(attr));
        op.set_attr_gep_src_elem_type(ctx, src_elem_type);
        op
    }

    /// Get the source pointer's element type.
    pub fn src_elem_type(&self, ctx: &Context) -> TypeHandle {
        self.get_attr_gep_src_elem_type(ctx)
            .expect("GetElementPtrOp missing or has incorrect src_elem_type attribute type")
            .get_type(ctx)
    }

    /// Get the indices of this GEP.
    pub fn indices(&self, ctx: &Context) -> Vec<GepIndex> {
        let op = &*self.op.deref(ctx);
        self.get_attr_gep_indices(ctx)
            .unwrap()
            .0
            .iter()
            .map(|index| match index {
                GepIndexAttr::Constant(c) => GepIndex::Constant(*c),
                GepIndexAttr::OperandIdx(i) => GepIndex::Value(op.get_operand(*i)),
            })
            .collect()
    }

    /// Returns the result element type of a GEP with the given source element type and indexes.
    /// See [getIndexedType](https://llvm.org/doxygen/classllvm_1_1GetElementPtrInst.html#a99d4bfe49182f8d80abb1960f2c12d46)
    pub fn indexed_type(
        ctx: &Context,
        src_elem_type: TypeHandle,
        indices: &[GepIndex],
    ) -> Result<TypeHandle> {
        fn indexed_type_inner(
            ctx: &Context,
            src_elem_type: TypeHandle,
            mut idx_itr: impl Iterator<Item = GepIndex>,
        ) -> Result<TypeHandle> {
            let Some(idx) = idx_itr.next() else {
                return Ok(src_elem_type);
            };
            let src_elem_type = &*src_elem_type.deref(ctx);
            if let Some(st) = src_elem_type.downcast_ref::<StructType>() {
                let GepIndex::Constant(i) = idx else {
                    return arg_err_noloc!(GetElementPtrOpErr::IndicesErr);
                };
                if st.is_opaque() || i as usize >= st.num_fields() {
                    return arg_err_noloc!(GetElementPtrOpErr::IndicesErr);
                }
                indexed_type_inner(ctx, st.field_type(i as usize), idx_itr)
            } else if let Some(at) = src_elem_type.downcast_ref::<ArrayType>() {
                indexed_type_inner(ctx, at.elem_type(), idx_itr)
            } else {
                arg_err_noloc!(GetElementPtrOpErr::IndicesErr)
            }
        }
        // The first index is for the base (source) pointer. Skip that.
        indexed_type_inner(ctx, src_elem_type, indices.iter().skip(1).cloned())
    }
}

#[derive(Error, Debug)]
pub enum LoadOpVerifyErr {
    #[error("Load operand must be a pointer")]
    OperandTypeErr,
}

/// Equivalent to LLVM's Load opcode.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `addr` | [PointerType] |
///
/// ### Result(s):
///
/// | result | description |
/// |-----|-------|
/// | `res` | sized LLVM type |
#[pliron_op(
    name = "llvm.load",
    format = "$0 ` ` opt_attr($llvm_alignment, $AlignmentAttr, label($align), delimiters(`[`, `]`)) ` : ` type($0)",
    interfaces = [
        OneResultInterface,
        OneOpdInterface,
        NResultsInterface<1>,
        NOpdsInterface<1>,
        AlignableOpInterface,
    ],
    operands = (address: PointerType),
    verifier = "succ"
)]
pub struct LoadOp;
impl LoadOp {
    /// Create a new [LoadOp]
    pub fn new(ctx: &mut Context, ptr: Value, res_ty: TypeHandle) -> Self {
        LoadOp {
            op: Operation::new(
                ctx,
                Self::get_concrete_op_info(),
                vec![res_ty],
                vec![ptr],
                vec![],
                0,
            ),
        }
    }
}

#[derive(Error, Debug)]
pub enum StoreOpVerifyErr {
    #[error("Store operand must have two operands")]
    NumOpdsErr,
    #[error("Store operand must have a pointer as its second argument")]
    AddrOpdTypeErr,
}

/// Equivalent to LLVM's Store opcode.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `addr` | [PointerType] |
/// | `value` | Sized type |
#[pliron_op(
    name = "llvm.store",
    format = "`*` $1 ` <- ` $0 ` ` opt_attr($llvm_alignment, $AlignmentAttr, label($align), delimiters(`[`, `]`))",
    interfaces = [
        NResultsInterface<0>,
        AlignableOpInterface,
        NOpdsInterface<2>
    ],
    operands = (value, address: PointerType),
    verifier = "succ"
)]
pub struct StoreOp;
impl StoreOp {
    /// Create a new [StoreOp]
    pub fn new(ctx: &mut Context, value: Value, ptr: Value) -> Self {
        StoreOp {
            op: Operation::new(
                ctx,
                Self::get_concrete_op_info(),
                vec![],
                vec![value, ptr],
                vec![],
                0,
            ),
        }
    }
}

/// Equivalent to LLVM's `atomicrmw`: atomically applies `kind` to the value at
/// a pointer and returns the old value.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `ptr` | [PointerType] |
/// | `val` | value to combine with the one in memory |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | the old value (same type as `val`) |
#[pliron_op(
    name = "llvm.atomicrmw",
    format = "attr($llvm_rmw_kind, $AtomicRmwKindAttr) ` ` $0 `, ` $1 ` ` opt_attr($llvm_rmw_syncscope, $StringAttr, label($syncscope)) attr($llvm_rmw_ordering, $AtomicOrderingAttr) ` : ` type($0)",
    interfaces = [
        OneResultInterface,
        NResultsInterface<1>,
        NOpdsInterface<2>,
    ],
    operands = (ptr: PointerType, val),
    attributes = (
        llvm_rmw_kind: AtomicRmwKindAttr,
        llvm_rmw_ordering: AtomicOrderingAttr,
        llvm_rmw_syncscope: StringAttr
    ),
    verifier = "succ"
)]
pub struct AtomicRmwOp;

impl AtomicRmwOp {
    /// Create a new [AtomicRmwOp]. `syncscope` is `None` for the system scope.
    pub fn new(
        ctx: &mut Context,
        ptr: Value,
        val: Value,
        kind: AtomicRmwKindAttr,
        ordering: AtomicOrderingAttr,
        syncscope: Option<String>,
    ) -> Self {
        use pliron::r#type::Typed;
        let res_ty = val.get_type(ctx);
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![res_ty],
            vec![ptr, val],
            vec![],
            0,
        );
        let op = AtomicRmwOp { op };
        op.set_attr_llvm_rmw_kind(ctx, kind);
        op.set_attr_llvm_rmw_ordering(ctx, ordering);
        if let Some(scope) = syncscope {
            op.set_attr_llvm_rmw_syncscope(ctx, StringAttr::new(scope));
        }
        op
    }
}

/// Equivalent to LLVM's `cmpxchg`: atomically compares the value at a pointer
/// with `cmp` and, if equal, stores `new`. Returns a `{ value, i1 }` pair of the
/// loaded value and whether the swap succeeded.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `ptr` | [PointerType] |
/// | `cmp` | expected value |
/// | `new` | value to store on success |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | `{ value, i1 }` (loaded value, success flag) |
#[pliron_op(
    name = "llvm.cmpxchg",
    format = "$0 `, ` $1 `, ` $2 ` ` opt_attr($llvm_cas_syncscope, $StringAttr, label($syncscope)) attr($llvm_cas_success_ordering, $AtomicOrderingAttr) ` ` attr($llvm_cas_failure_ordering, $AtomicOrderingAttr) ` : ` type($0)",
    interfaces = [
        OneResultInterface,
        NResultsInterface<1>,
        NOpdsInterface<3>,
    ],
    operands = (ptr: PointerType, cmp, new_val),
    attributes = (
        llvm_cas_success_ordering: AtomicOrderingAttr,
        llvm_cas_failure_ordering: AtomicOrderingAttr,
        llvm_cas_syncscope: StringAttr
    ),
    verifier = "succ"
)]
pub struct AtomicCmpxchgOp;

impl AtomicCmpxchgOp {
    /// Create a new [AtomicCmpxchgOp]. `syncscope` is `None` for the system scope.
    pub fn new(
        ctx: &mut Context,
        ptr: Value,
        cmp: Value,
        new_val: Value,
        success_ordering: AtomicOrderingAttr,
        failure_ordering: AtomicOrderingAttr,
        syncscope: Option<String>,
    ) -> Self {
        use pliron::r#type::Typed;
        let val_ty = cmp.get_type(ctx);
        let bool_ty = IntegerType::get(ctx, 1, Signedness::Signless);
        let res_ty = StructType::get_unnamed(ctx, vec![val_ty, bool_ty.into()]).into();
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![res_ty],
            vec![ptr, cmp, new_val],
            vec![],
            0,
        );
        let op = AtomicCmpxchgOp { op };
        op.set_attr_llvm_cas_success_ordering(ctx, success_ordering);
        op.set_attr_llvm_cas_failure_ordering(ctx, failure_ordering);
        if let Some(scope) = syncscope {
            op.set_attr_llvm_cas_syncscope(ctx, StringAttr::new(scope));
        }
        op
    }
}

/// Equivalent to LLVM's `fence`: orders memory accesses. Has no operands or
/// results.
#[pliron_op(
    name = "llvm.fence",
    format = "opt_attr($llvm_fence_syncscope, $StringAttr, label($syncscope)) attr($llvm_fence_ordering, $AtomicOrderingAttr)",
    interfaces = [NResultsInterface<0>, NOpdsInterface<0>],
    attributes = (llvm_fence_ordering: AtomicOrderingAttr, llvm_fence_syncscope: StringAttr),
    verifier = "succ"
)]
pub struct FenceOp;

impl FenceOp {
    /// Create a new [FenceOp]. `syncscope` is `None` for the system scope.
    pub fn new(ctx: &mut Context, ordering: AtomicOrderingAttr, syncscope: Option<String>) -> Self {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        let op = FenceOp { op };
        op.set_attr_llvm_fence_ordering(ctx, ordering);
        if let Some(scope) = syncscope {
            op.set_attr_llvm_fence_syncscope(ctx, StringAttr::new(scope));
        }
        op
    }
}

/// Equivalent to LLVM's atomic `load`.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `ptr` | [PointerType] |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | the loaded value |
#[pliron_op(
    name = "llvm.atomic_load",
    format = "$0 ` ` opt_attr($llvm_alignment, $AlignmentAttr, label($align), delimiters(`[`, `]`)) ` ` opt_attr($llvm_ld_syncscope, $StringAttr, label($syncscope)) attr($llvm_ld_ordering, $AtomicOrderingAttr) ` : ` type($0)",
    interfaces = [
        OneResultInterface,
        OneOpdInterface,
        NResultsInterface<1>,
        NOpdsInterface<1>,
        AlignableOpInterface,
    ],
    operands = (ptr: PointerType),
    attributes = (llvm_ld_ordering: AtomicOrderingAttr, llvm_ld_syncscope: StringAttr),
    verifier = "succ"
)]
pub struct AtomicLoadOp;

impl AtomicLoadOp {
    /// Create a new [AtomicLoadOp]. `syncscope` is `None` for the system scope.
    pub fn new(
        ctx: &mut Context,
        ptr: Value,
        res_ty: TypeHandle,
        ordering: AtomicOrderingAttr,
        syncscope: Option<String>,
    ) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![res_ty],
            vec![ptr],
            vec![],
            0,
        );
        let op = AtomicLoadOp { op };
        op.set_attr_llvm_ld_ordering(ctx, ordering);
        if let Some(scope) = syncscope {
            op.set_attr_llvm_ld_syncscope(ctx, StringAttr::new(scope));
        }
        op
    }
}

/// Equivalent to LLVM's atomic `store`.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `val` | value to store |
/// | `ptr` | [PointerType] |
#[pliron_op(
    name = "llvm.atomic_store",
    format = "`*` $1 ` <- ` $0 ` ` opt_attr($llvm_alignment, $AlignmentAttr, label($align), delimiters(`[`, `]`)) ` ` opt_attr($llvm_st_syncscope, $StringAttr, label($syncscope)) attr($llvm_st_ordering, $AtomicOrderingAttr)",
    interfaces = [
        NResultsInterface<0>,
        AlignableOpInterface,
        NOpdsInterface<2>
    ],
    operands = (value, ptr: PointerType),
    attributes = (llvm_st_ordering: AtomicOrderingAttr, llvm_st_syncscope: StringAttr),
    verifier = "succ"
)]
pub struct AtomicStoreOp;

impl AtomicStoreOp {
    /// Create a new [AtomicStoreOp]. `syncscope` is `None` for the system scope.
    pub fn new(
        ctx: &mut Context,
        value: Value,
        ptr: Value,
        ordering: AtomicOrderingAttr,
        syncscope: Option<String>,
    ) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![],
            vec![value, ptr],
            vec![],
            0,
        );
        let op = AtomicStoreOp { op };
        op.set_attr_llvm_st_ordering(ctx, ordering);
        if let Some(scope) = syncscope {
            op.set_attr_llvm_st_syncscope(ctx, StringAttr::new(scope));
        }
        op
    }
}

/// Equivalent to LLVM's inline assembly call. The template and constraint
/// strings follow LLVM's inline-asm syntax; `convergent` marks asm that must
/// not be reordered across divergent control flow (e.g. warp-synchronous PTX).
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `inputs` | input operands to the inline asm |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | the asm result (a void type when there is none) |
#[pliron_op(
    name = "llvm.inline_asm",
    format = "attr($inline_asm_template, $StringAttr) `, ` attr($inline_asm_constraints, $StringAttr) ` convergent = ` attr($inline_asm_convergent, $BoolAttr) ` (` operands(CharSpace(`,`)) `) : ` type($0)",
    interfaces = [OneResultInterface, NResultsInterface<1>],
    attributes = (
        inline_asm_template: StringAttr,
        inline_asm_constraints: StringAttr,
        inline_asm_convergent: BoolAttr
    ),
    verifier = "succ"
)]
pub struct InlineAsmOp;

impl InlineAsmOp {
    /// Create a new [InlineAsmOp]. Use a void result type for asm with no
    /// result value.
    pub fn new(
        ctx: &mut Context,
        result_ty: TypeHandle,
        inputs: Vec<Value>,
        asm_template: &str,
        constraints: &str,
        convergent: bool,
    ) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_ty],
            inputs,
            vec![],
            0,
        );
        let op = InlineAsmOp { op };
        op.set_attr_inline_asm_template(ctx, StringAttr::new(asm_template.to_string()));
        op.set_attr_inline_asm_constraints(ctx, StringAttr::new(constraints.to_string()));
        op.set_attr_inline_asm_convergent(ctx, BoolAttr::new(convergent));
        op
    }
}

/// Equivalent to LLVM's Store opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `callee_operands` | Optional function pointer followed by any number of parameters |
///
/// ### Result(s):
///
/// | result | description |
/// |-----|-------|
/// | `res` | LLVM type |
#[pliron_op(
    name = "llvm.call",
    interfaces = [OneResultInterface, NResultsInterface<1>],
    attributes = (llvm_call_callee: IdentifierAttr, llvm_call_fastmath_flags: FastmathFlagsAttr)
)]
pub struct CallOp;

impl CallOp {
    /// Get a new [CallOp].
    pub fn new(
        ctx: &mut Context,
        callee: CallOpCallable,
        callee_ty: TypedHandle<FuncType>,
        mut args: Vec<Value>,
    ) -> Self {
        let res_ty = callee_ty.deref(ctx).result_type();
        let op = match callee {
            CallOpCallable::Direct(cval) => {
                let op = Operation::new(
                    ctx,
                    Self::get_concrete_op_info(),
                    vec![res_ty],
                    args,
                    vec![],
                    0,
                );
                let op = CallOp { op };
                op.set_attr_llvm_call_callee(ctx, IdentifierAttr::new(cval));
                op
            }
            CallOpCallable::Indirect(csym) => {
                args.insert(0, csym);
                let op = Operation::new(
                    ctx,
                    Self::get_concrete_op_info(),
                    vec![res_ty],
                    args,
                    vec![],
                    0,
                );
                CallOp { op }
            }
        };
        op.set_callee_type(ctx, callee_ty.into());
        op
    }
}

#[derive(Error, Debug)]
pub enum SymbolUserOpVerifyErr {
    #[error("Symbol {0} not found")]
    SymbolNotFound(String),
    #[error("Function {0} should have been llvm.func type")]
    NotLlvmFunc(String),
    #[error("AddressOf Op can only refer to a function or a global variable")]
    AddressOfInvalidReference,
    #[error("Function call has incorrect type: {0}")]
    FuncTypeErr(String),
}

#[op_interface_impl]
impl SymbolUserOpInterface for CallOp {
    fn verify_symbol_uses(
        &self,
        ctx: &Context,
        symbol_tables: &mut SymbolTableCollection,
    ) -> Result<()> {
        match self.callee(ctx) {
            CallOpCallable::Direct(callee_sym) => {
                let Some(callee) = symbol_tables.lookup_symbol_in_nearest_table(
                    ctx,
                    self.get_operation(),
                    &callee_sym,
                ) else {
                    return verify_err!(
                        self.loc(ctx),
                        SymbolUserOpVerifyErr::SymbolNotFound(callee_sym.to_string())
                    );
                };
                let Some(func_op) = (&*callee as &dyn Op).downcast_ref::<FuncOp>() else {
                    return verify_err!(
                        self.loc(ctx),
                        SymbolUserOpVerifyErr::NotLlvmFunc(callee_sym.to_string())
                    );
                };
                let func_op_ty = func_op.get_type(ctx);

                if func_op_ty.to_handle() != self.callee_type(ctx) {
                    return verify_err!(
                        self.loc(ctx),
                        SymbolUserOpVerifyErr::FuncTypeErr(format!(
                            "expected {}, got {}",
                            func_op_ty.disp(ctx),
                            self.callee_type(ctx).disp(ctx)
                        ))
                    );
                }
            }
            CallOpCallable::Indirect(pointer) => {
                use pliron::r#type::Typed;
                if !pointer.get_type(ctx).deref(ctx).is::<PointerType>() {
                    return verify_err!(
                        self.loc(ctx),
                        SymbolUserOpVerifyErr::FuncTypeErr("Callee must be a pointer".to_string())
                    );
                }
            }
        }
        Ok(())
    }

    fn used_symbols(&self, ctx: &Context) -> Vec<Identifier> {
        match self.callee(ctx) {
            CallOpCallable::Direct(identifier) => vec![identifier],
            CallOpCallable::Indirect(_) => vec![],
        }
    }
}

#[op_interface_impl]
impl CallOpInterface for CallOp {
    fn callee(&self, ctx: &Context) -> CallOpCallable {
        let op = self.op.deref(ctx);
        if let Some(callee_sym) = self.get_attr_llvm_call_callee(ctx) {
            CallOpCallable::Direct(callee_sym.clone().into())
        } else {
            assert!(
                op.get_num_operands() > 0,
                "Indirect call must have function pointer operand"
            );
            CallOpCallable::Indirect(op.get_operand(0))
        }
    }

    fn args(&self, ctx: &Context) -> Vec<Value> {
        let op = self.op.deref(ctx);
        // If this is an indirect call, the first operand is the callee value.
        let skip = if matches!(self.callee(ctx), CallOpCallable::Direct(_)) {
            0
        } else {
            1
        };
        op.operands().skip(skip).collect()
    }
}

impl Printable for CallOp {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &pliron::printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let callee = self.callee(ctx);
        write!(
            f,
            "{} = {} ",
            self.get_result(ctx).disp(ctx),
            self.get_opid()
        )?;
        match callee {
            CallOpCallable::Direct(callee_sym) => {
                write!(f, "@{callee_sym}")?;
            }
            CallOpCallable::Indirect(callee_val) => {
                write!(f, "{}", callee_val.disp(ctx))?;
            }
        }

        if let Some(fmf) = self.get_attr_llvm_call_fastmath_flags(ctx)
            && *fmf != FastmathFlagsAttr::default()
        {
            write!(f, " {}", fmf.disp(ctx))?;
        }

        let args = self.args(ctx);
        let ty = self.callee_type(ctx);
        write!(
            f,
            " ({}) : {}",
            list_with_sep(&args, pliron::printable::ListSeparator::CharSpace(',')).disp(ctx),
            ty.disp(ctx)
        )?;
        Ok(())
    }
}

impl Parsable for CallOp {
    type Arg = Vec<(Identifier, Location)>;
    type Parsed = OpObj;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        results: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let direct_callee = combine::token('@')
            .with(Identifier::parser(()))
            .map(CallOpCallable::Direct);
        let indirect_callee = ssa_opd_parser().map(CallOpCallable::Indirect);
        let callee_parser = direct_callee.or(indirect_callee);
        let fastmath_flags_parser = optional(FastmathFlagsAttr::parser(()));
        let args_parser = delimited_list_parser('(', ')', ',', ssa_opd_parser());
        let ty_parser = spaced(combine::token(':')).with(TypedHandle::<FuncType>::parser(()));

        let mut final_parser = spaced(callee_parser)
            .and(spaced(fastmath_flags_parser))
            .and(spaced(args_parser))
            .and(ty_parser)
            .then(move |(((callee, fastmath_flags), args), ty)| {
                let results = results.clone();
                combine::parser(move |parsable_state: &mut StateStream<'a>| {
                    let ctx = &mut parsable_state.state.ctx;
                    let op = CallOp::new(ctx, callee.clone(), ty, args.clone());
                    if let Some(fmf) = &fastmath_flags {
                        op.set_attr_llvm_call_fastmath_flags(ctx, *fmf);
                    }
                    process_parsed_ssa_defs(parsable_state, &results, op.get_operation())?;
                    Ok(OpObj::new(op)).into_parse_result()
                })
            });

        final_parser.parse_stream(state_stream).into_result()
    }
}

impl Verify for CallOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        // Check that the argument and result types match the callee type.
        let callee_ty = &*self.callee_type(ctx).deref(ctx);
        let Some(callee_ty) = callee_ty.downcast_ref::<FuncType>() else {
            return verify_err!(
                self.loc(ctx),
                SymbolUserOpVerifyErr::FuncTypeErr("Callee is not a function".to_string())
            );
        };
        // Check the function type against the arguments.
        let args = self.args(ctx);
        let expected_args = callee_ty.arg_types();
        if !callee_ty.is_var_arg() && args.len() != expected_args.len() {
            return verify_err!(
                self.loc(ctx),
                SymbolUserOpVerifyErr::FuncTypeErr("argument count mismatch.".to_string())
            );
        }
        use pliron::r#type::Typed;
        for (arg_idx, (arg, expected_arg)) in args.iter().zip(expected_args.iter()).enumerate() {
            if arg.get_type(ctx) != *expected_arg {
                return verify_err!(
                    self.loc(ctx),
                    SymbolUserOpVerifyErr::FuncTypeErr(format!(
                        "argument {} type mismatch: expected {}, got {}",
                        arg_idx,
                        expected_arg.disp(ctx),
                        arg.get_type(ctx).disp(ctx)
                    ))
                );
            }
        }

        if callee_ty.result_type() != self.result_type(ctx) {
            return verify_err!(
                self.loc(ctx),
                SymbolUserOpVerifyErr::FuncTypeErr(format!(
                    "result type mismatch: expected {}, got {}",
                    callee_ty.result_type().disp(ctx),
                    self.result_type(ctx).disp(ctx)
                ))
            );
        }

        Ok(())
    }
}

/// Constant value operation for the LLVM dialect.
/// Similar to MLIR's [llvm.mlir.constant](https://mlir.llvm.org/docs/Dialects/LLVM/#llvmmlirconstant-llvmconstantop).
///
/// This operation represents a constant value that must be an integer or float attribute.
/// Unlike `builtin.constant` which supports any attribute type, `llvm.constant` enforces
/// that the value must be either an integer or float.
///
/// ### Results:
///
/// | result | description |
/// |-----|-------|
/// | `result` | integer or float type |
#[pliron_op(
    name = "llvm.constant",
    format = "`<` $llvm_constant_value `>` ` : ` type($0)",
    interfaces = [NOpdsInterface<0>, OneResultInterface, NResultsInterface<1>],
    attributes = (llvm_constant_value),
)]
pub struct ConstantOp;

impl ConstantOp {
    /// Get the constant value that this Op defines.
    pub fn get_value(&self, ctx: &Context) -> AttrObj {
        self.get_attr_llvm_constant_value(ctx).unwrap().clone()
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
        op.set_attr_llvm_constant_value(ctx, value);
        op
    }
}

#[derive(Error, Debug)]
#[error("{}: Unexpected type", ConstantOp::get_opid_static())]
pub enum ConstantOpVerifyErr {
    #[error("ConstantOp must have either an integer or a float value")]
    InvalidValue,
}

impl Verify for ConstantOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);
        let value = self.get_value(ctx);
        if !(value.is::<IntegerAttr>() || attr_impls::<dyn FloatAttr>(&*value)) {
            return verify_err!(loc, ConstantOpVerifyErr::InvalidValue)?;
        }
        Ok(())
    }
}

/// Undefined value of a type.
/// See MLIR's [llvm.mlir.undef](https://mlir.llvm.org/docs/Dialects/LLVM/#llvmmlirundef-llvmundefop).
///
/// ### Results:
/// | result | description |
/// |-----|-------|
/// | `result` | any type |
#[pliron_op(
    name = "llvm.undef",
    format = "`: ` type($0)",
    interfaces = [OneResultInterface, NResultsInterface<1>, NOpdsInterface<0>],
    verifier = "succ"
)]
pub struct UndefOp;

impl UndefOp {
    /// Create a new [UndefOp].
    pub fn new(ctx: &mut Context, result_ty: TypeHandle) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_ty],
            vec![],
            vec![],
            0,
        );
        UndefOp { op }
    }
}

/// Poison value of a type.
/// See MLIR's [llvm.mlir.poison](https://mlir.llvm.org/docs/Dialects/LLVM/#llvmmlirpoison-llvmpoisonop).
///
/// ### Results:
/// | result | description |
/// |-----|-------|
/// | `result` | any type |
#[pliron_op(
    name = "llvm.poison",
    format = "`: ` type($0)",
    interfaces = [OneResultInterface, NResultsInterface<1>],
    verifier = "succ"
)]
pub struct PoisonOp;

impl PoisonOp {
    /// Create a new [PoisonOp].
    pub fn new(ctx: &mut Context, result_ty: TypeHandle) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_ty],
            vec![],
            vec![],
            0,
        );
        PoisonOp { op }
    }
}

/// Freeze value of a type.
/// See MLIR's [llvm.mlir.freeze](https://mlir.llvm.org/docs/Dialects/LLVM/#llvmfreeze-llvmfreezeop).
///
/// ### Results:
/// | result | description |
/// |-----|-------|
/// | `result` | any type |
///
/// ### Operands:
/// | operand | description |
/// |-----|-------|
/// | `value` | any type |
#[pliron_op(
    name = "llvm.freeze",
    format = "$0 ` : ` type($0)",
    interfaces = [OneOpdInterface, OneResultInterface, NOpdsInterface<1>, NResultsInterface<1>],
    verifier = "succ"
)]
pub struct FreezeOp;

impl FreezeOp {
    /// Create a new [FreezeOp].
    pub fn new(ctx: &mut Context, value: Value) -> Self {
        use pliron::r#type::Typed;
        let result_ty = value.get_type(ctx);
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_ty],
            vec![value],
            vec![],
            0,
        );
        FreezeOp { op }
    }
}

/// Same as MLIR's LLVM dialect [ZeroOp](https://mlir.llvm.org/docs/Dialects/LLVM/#llvmmlirzero-llvmzeroop)
/// It creates a zero-initialized value of the specified LLVM IR dialect type.
/// Results:
///
/// | result | description |
/// |-----|-------|
/// | `result` | any type |
#[pliron_op(
    name = "llvm.zero",
    format = "`: ` type($0)",
    interfaces = [NOpdsInterface<0>, OneResultInterface, NResultsInterface<1>],
    verifier = "succ"
)]
pub struct ZeroOp;

impl ZeroOp {
    /// Create a new [ZeroOp].
    pub fn new(ctx: &mut Context, result_ty: TypeHandle) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_ty],
            vec![],
            vec![],
            0,
        );
        ZeroOp { op }
    }
}

#[derive(Error, Debug)]
pub enum GlobalOpVerifyErr {
    #[error("GlobalOp must have a type")]
    MissingType,
}

/// Same as MLIR's LLVM dialect [GlobalOp](https://mlir.llvm.org/docs/Dialects/LLVM/#llvmmlirglobal-llvmglobalop)
/// It creates a global variable of the specified LLVM IR dialect type.
/// An initializer can be specified either as an attribute or in the
/// operation's initializer region, ending with a return.
#[pliron_op(
    name = "llvm.global",
    interfaces = [
        IsolatedFromAboveInterface,
        NOpdsInterface<0>,
        NResultsInterface<0>,
        SymbolOpInterface,
        SingleBlockRegionInterface,
        LlvmSymbolName,
        AlignableOpInterface
    ],
    attributes = (
        llvm_global_type: TypeAttr,
        global_initializer,
        llvm_global_linkage: LinkageAttr,
        llvm_global_addrspace: AddressSpaceAttr
    )
)]
pub struct GlobalOp;

impl GlobalOp {
    /// Create a new [GlobalOp]. An initializer region can be added later if needed.
    pub fn new(ctx: &mut Context, name: Identifier, ty: TypeHandle) -> Self {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        let op = GlobalOp { op };
        op.set_symbol_name(ctx, name);
        op.set_attr_llvm_global_type(ctx, TypeAttr::new(ty));
        op
    }

    /// Get the address space of this global (0 if unset).
    pub fn address_space(&self, ctx: &Context) -> u32 {
        self.get_attr_llvm_global_addrspace(ctx)
            .map_or(0, |attr| attr.0)
    }

    /// Set the address space of this global.
    pub fn set_address_space(&self, ctx: &mut Context, addr_space: u32) {
        self.set_attr_llvm_global_addrspace(ctx, AddressSpaceAttr(addr_space));
    }
}

impl pliron::r#type::Typed for GlobalOp {
    fn get_type(&self, ctx: &Context) -> TypeHandle {
        pliron::r#type::Typed::get_type(
            &*self
                .get_attr_llvm_global_type(ctx)
                .expect("GlobalOp missing or has incorrect type attribute"),
            ctx,
        )
    }
}

impl GlobalOp {
    /// Get the initializer value of this global variable.
    pub fn get_initializer_value(&self, ctx: &Context) -> Option<AttrObj> {
        self.get_attr_global_initializer(ctx).map(|v| v.clone())
    }

    /// Get the initializer region's block of this global variable.
    /// This is a block that ends with a return operation.
    /// The return operation must have the same type as the global variable.
    pub fn get_initializer_block(&self, ctx: &Context) -> Option<Ptr<BasicBlock>> {
        (self.op.deref(ctx).num_regions() > 0).then(|| self.get_body(ctx, 0))
    }

    /// Get the initializer region of this global variable.
    pub fn get_initializer_region(&self, ctx: &Context) -> Option<Ptr<Region>> {
        (self.op.deref(ctx).num_regions() > 0)
            .then(|| self.get_operation().deref(ctx).get_region(0))
    }

    /// Set a simple initializer value for this global variable.
    pub fn set_initializer_value(&self, ctx: &Context, value: AttrObj) {
        self.set_attr_global_initializer(ctx, value);
    }

    /// Add an initializer region (with an entry block) for this global variable.
    /// There shouldn't already be one.
    pub fn add_initializer_region(&self, ctx: &mut Context) -> Ptr<Region> {
        assert!(
            self.get_initializer_value(ctx).is_none(),
            "Attempt to create an initializer region when there already is an initializer value"
        );
        let region = Operation::add_region(self.get_operation(), ctx);
        let entry = BasicBlock::new(ctx, Some("entry".try_into().unwrap()), vec![]);
        entry.insert_at_front(region, ctx);

        region
    }
}

impl IsDeclaration for GlobalOp {
    fn is_declaration(&self, ctx: &Context) -> bool {
        self.get_initializer_value(ctx).is_none() && self.get_initializer_region(ctx).is_none()
    }
}

impl Verify for GlobalOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);

        // The name must be set. That is checked by the SymbolOpInterface.
        // So we check that other attributes are set. Start with type.
        if self.get_attr_llvm_global_type(ctx).is_none() {
            return verify_err!(loc, GlobalOpVerifyErr::MissingType);
        }

        // Check that there is at most one initializer
        if self.get_initializer_value(ctx).is_some() && self.get_initializer_region(ctx).is_some() {
            return verify_err!(loc, GlobalOpVerifyErr::MissingType);
        }

        Ok(())
    }
}

impl Printable for GlobalOp {
    fn fmt(
        &self,
        ctx: &Context,
        state: &pliron::printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(
            f,
            "{} @{} : {}",
            self.get_opid(),
            self.get_symbol_name(ctx),
            <Self as pliron::r#type::Typed>::get_type(self, ctx).disp(ctx)
        )?;

        // Print attributes except for type, initializer and symbol name.
        let mut attributes_to_print_separately =
            self.op.deref(ctx).attributes.clone_skip_outlined();
        attributes_to_print_separately.0.retain(|key, _| {
            key != &*ATTR_KEY_LLVM_GLOBAL_TYPE
                && key != &*ATTR_KEY_SYM_NAME
                && key != &*ATTR_KEY_GLOBAL_INITIALIZER
        });
        indented_block!(state, {
            write!(
                f,
                "{}{}",
                indented_nl(state),
                attributes_to_print_separately.disp(ctx)
            )?;
        });

        if let Some(init_value) = self.get_initializer_value(ctx) {
            write!(f, " = {}", init_value.disp(ctx))?;
        }

        if let Some(init_region) = self.get_initializer_region(ctx) {
            write!(f, " = {}", init_region.print(ctx, state))?;
        }

        Ok(())
    }
}

impl Parsable for GlobalOp {
    type Arg = Vec<(Identifier, Location)>;
    type Parsed = OpObj;
    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        results: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let loc = state_stream.loc();
        if !results.is_empty() {
            return input_err!(loc, "GlobalOp must cannot have results")?;
        }
        let name_parser = combine::token('@').with(Identifier::parser(()));
        let type_parser = type_parser();
        let attr_dict_parser = AttributeDict::parser(());

        let mut parser = name_parser
            .skip(spaced(combine::token(':')))
            .and(type_parser)
            .and(spaced(attr_dict_parser));

        let (((name, ty), attr_dict), _) = parser.parse_stream(state_stream).into_result()?;
        let op = GlobalOp::new(state_stream.state.ctx, name, ty);
        op.get_operation()
            .deref_mut(state_stream.state.ctx)
            .attributes
            .0
            .extend(attr_dict.0);

        enum Initializer {
            Value(AttrObj),
            Region(Ptr<Region>),
        }
        // Parse optional initializer value or region.
        let initializer_parser = combine::token('=').skip(spaces()).with(
            attr_parser()
                .map(Initializer::Value)
                .or(Region::parser(op.get_operation()).map(Initializer::Region)),
        );

        let initializer = spaces()
            .with(combine::optional(initializer_parser))
            .parse_stream(state_stream)
            .into_result()?;

        if let Some(initializer) = initializer.0 {
            match initializer {
                Initializer::Value(v) => op.set_initializer_value(state_stream.state.ctx, v),
                Initializer::Region(_r) => {
                    // Nothing to do since the region is already added to the operation during parsing.
                }
            }
        }

        Ok(OpObj::new(op)).into_parse_result()
    }
}

/// Same as MLIR's LLVM dialect [AddressOfOp](https://mlir.llvm.org/docs/Dialects/LLVM/#llvmmliraddressof-llvmaddressofop).
/// Creates an SSA value containing a pointer to a global value (function, variable etc).
///
/// ### Results:
///
/// | result | description |
/// |-----|-------|
/// | `result` | LLVM pointer type |
///
#[pliron_op(
    name = "llvm.addressof",
    format = "`@` attr($global_name, $IdentifierAttr) ` : ` type($0)",
    interfaces = [OneResultInterface, NResultsInterface<1>, NOpdsInterface<0>],
    results = (_: PointerType),
    attributes = (global_name: IdentifierAttr),
)]
pub struct AddressOfOp;

#[derive(Error, Debug)]
enum AddressOfOpVerifyErr {
    #[error("AddressOfOp is missing its `global_name` attribute")]
    MissingGlobalName,
}

impl Verify for AddressOfOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        if self.get_attr_global_name(ctx).is_none() {
            verify_err!(self.loc(ctx), AddressOfOpVerifyErr::MissingGlobalName)?
        }
        Ok(())
    }
}

impl AddressOfOp {
    /// Create a new [AddressOfOp].
    pub fn new(ctx: &mut Context, global_name: Identifier, address_space: u32) -> Self {
        let result_type = PointerType::get(ctx, address_space).into();
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_type],
            vec![],
            vec![],
            0,
        );
        let op = AddressOfOp { op };
        op.set_attr_global_name(ctx, IdentifierAttr::new(global_name));
        op
    }

    /// Get the global name that this refers to.
    pub fn get_global_name(&self, ctx: &Context) -> Identifier {
        self.get_attr_global_name(ctx)
            .expect("AddressOfOp missing or has incorrect global_name attribute type")
            .clone()
            .into()
    }

    /// If this operation referes to a global, get it.
    pub fn get_global(
        &self,
        ctx: &Context,
        symbol_tables: &mut SymbolTableCollection,
    ) -> Option<GlobalOp> {
        let global_name = self.get_global_name(ctx);
        symbol_tables
            .lookup_symbol_in_nearest_table(ctx, self.get_operation(), &global_name)
            .and_then(|sym_op| {
                (sym_op as Box<dyn Op>)
                    .downcast::<GlobalOp>()
                    .map(|op| *op)
                    .ok()
            })
    }

    /// If this operation refers to a function, get it.
    pub fn get_function(
        &self,
        ctx: &Context,
        symbol_tables: &mut SymbolTableCollection,
    ) -> Option<FuncOp> {
        let global_name = self.get_global_name(ctx);
        symbol_tables
            .lookup_symbol_in_nearest_table(ctx, self.get_operation(), &global_name)
            .and_then(|sym_op| {
                (sym_op as Box<dyn Op>)
                    .downcast::<FuncOp>()
                    .map(|op| *op)
                    .ok()
            })
    }
}

#[op_interface_impl]
impl SymbolUserOpInterface for AddressOfOp {
    fn used_symbols(&self, ctx: &Context) -> Vec<Identifier> {
        vec![self.get_global_name(ctx)]
    }

    fn verify_symbol_uses(
        &self,
        ctx: &Context,
        symbol_tables: &mut SymbolTableCollection,
    ) -> Result<()> {
        let loc = self.loc(ctx);
        let global_name = self.get_global_name(ctx);
        let Some(symbol) =
            symbol_tables.lookup_symbol_in_nearest_table(ctx, self.get_operation(), &global_name)
        else {
            return verify_err!(
                loc,
                SymbolUserOpVerifyErr::SymbolNotFound(global_name.to_string())
            );
        };

        // Symbol can only be a FuncOp or a GlobalOp
        let is_global = (&*symbol as &dyn Op).is::<GlobalOp>();
        let is_func = (&*symbol as &dyn Op).is::<FuncOp>();
        if !is_global && !is_func {
            return verify_err!(loc, SymbolUserOpVerifyErr::AddressOfInvalidReference);
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
enum IntCastVerifyErr {
    #[error("Result must be an integer")]
    ResultTypeErr,
    #[error("Operand must be an integer")]
    OperandTypeErr,
    #[error("Result type must be larger than operand type")]
    ResultTypeSmallerThanOperand,
    #[error("Result type must be smaller than operand type")]
    ResultTypeLargerThanOperand,
    #[error("Result type must be equal to operand type")]
    ResultTypeEqualToOperand,
}

/// Ensure that the integer cast operation is valid.
/// This checks that the result type is an integer and that the operand type is also an integer.
/// It also checks that the result type is larger or smaller than the operand type (`cmp` operand).
fn integer_cast_verify(op: &Operation, ctx: &Context, cmp: ICmpPredicateAttr) -> Result<()> {
    use pliron::r#type::Typed;

    let loc = op.loc();
    let mut res_ty = op.get_type(0).deref(ctx);
    let mut opd_ty = op.get_operand(0).get_type(ctx).deref(ctx);

    if let Some(vec_res_ty) = res_ty.downcast_ref::<VectorType>() {
        res_ty = vec_res_ty.elem_type().deref(ctx);
    }
    if let Some(vec_opd_ty) = opd_ty.downcast_ref::<VectorType>() {
        opd_ty = vec_opd_ty.elem_type().deref(ctx);
    }

    let Some(res_ty) = res_ty.downcast_ref::<IntegerType>() else {
        return verify_err!(loc, IntCastVerifyErr::ResultTypeErr);
    };
    let Some(opd_ty) = opd_ty.downcast_ref::<IntegerType>() else {
        return verify_err!(loc, IntCastVerifyErr::OperandTypeErr);
    };

    match cmp {
        ICmpPredicateAttr::SLT | ICmpPredicateAttr::ULT => {
            if res_ty.width() >= opd_ty.width() {
                return verify_err!(loc, IntCastVerifyErr::ResultTypeLargerThanOperand);
            }
        }
        ICmpPredicateAttr::SGT | ICmpPredicateAttr::UGT => {
            if res_ty.width() <= opd_ty.width() {
                return verify_err!(loc, IntCastVerifyErr::ResultTypeSmallerThanOperand);
            }
        }
        ICmpPredicateAttr::SLE | ICmpPredicateAttr::ULE => {
            if res_ty.width() > opd_ty.width() {
                return verify_err!(loc, IntCastVerifyErr::ResultTypeLargerThanOperand);
            }
        }
        ICmpPredicateAttr::SGE | ICmpPredicateAttr::UGE => {
            if res_ty.width() < opd_ty.width() {
                return verify_err!(loc, IntCastVerifyErr::ResultTypeSmallerThanOperand);
            }
        }
        ICmpPredicateAttr::EQ | ICmpPredicateAttr::NE => {
            if res_ty.width() != opd_ty.width() {
                return verify_err!(loc, IntCastVerifyErr::ResultTypeEqualToOperand);
            }
        }
    }
    Ok(())
}

/// Equivalent to LLVM's sext opcode.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | Signless integer |
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | Signless integer |
#[pliron_op(
    name = "llvm.sext",
    format = "$0 ` to ` type($0)",
    interfaces = [CastOpInterface, OneResultInterface, OneOpdInterface, NResultsInterface<1>, NOpdsInterface<1>]
)]
pub struct SExtOp;
impl Verify for SExtOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        integer_cast_verify(
            &self.get_operation().deref(ctx),
            ctx,
            ICmpPredicateAttr::SGT,
        )
    }
}

/// Equivalent to LLVM's zext opcode.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | Signless integer |
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | Signless integer |
#[pliron_op(
    name = "llvm.zext",
    format = "`<nneg=` attr($llvm_nneg_flag, `pliron::builtin::attributes::BoolAttr`) `> ` $0 ` to ` type($0)",
    interfaces = [
        CastOpInterface,
        OneResultInterface,
        OneOpdInterface,
        NNegFlag,
        CastOpWithNNegInterface,
        NResultsInterface<1>,
        NOpdsInterface<1>
    ]
)]
pub struct ZExtOp;

impl Verify for ZExtOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        integer_cast_verify(
            &self.get_operation().deref(ctx),
            ctx,
            ICmpPredicateAttr::UGT,
        )
    }
}

/// Equivalent to LLVM's FPExt opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | Floating-point number |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | Floating-point number |
#[pliron_op(
    name = "llvm.fpext",
    format = "attr($llvm_fast_math_flags, $FastmathFlagsAttr) ` ` $0 ` to ` type($0)",
    interfaces = [CastOpInterface, OneResultInterface, OneOpdInterface, FastMathFlags, NResultsInterface<1>, NOpdsInterface<1>]
)]
pub struct FPExtOp;

impl Verify for FPExtOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        // Check operand type to be a float
        let opd_ty = OneOpdInterface::operand_type(self, ctx).deref(ctx);
        let Some(opd_float_ty) = type_cast::<dyn FloatTypeInterface>(&*opd_ty) else {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::OperandTypeErr);
        };
        let res_ty = OneResultInterface::result_type(self, ctx).deref(ctx);
        let Some(res_float_ty) = type_cast::<dyn FloatTypeInterface>(&*res_ty) else {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::ResultTypeErr);
        };

        let opd_size = opd_float_ty.get_semantics().bits;
        let res_size = res_float_ty.get_semantics().bits;
        if res_size <= opd_size {
            return verify_err!(
                self.loc(ctx),
                FloatCastVerifyErr::ResultTypeSmallerThanOperand
            );
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum FloatCastVerifyErr {
    #[error("Incorrect operand type")]
    OperandTypeErr,
    #[error("Incorrect result type")]
    ResultTypeErr,
    #[error("Operand and result must both be scalars or vectors with matching shape")]
    MismatchedVectorShape,
    #[error("Result type must be bigger than the operand type")]
    ResultTypeSmallerThanOperand,
    #[error("Operand type must be bigger than the result type")]
    OperandTypeSmallerThanResult,
}

/// Equivalent to LLVM's trunc opcode.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | Signless integer |
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | Signless integer |
#[pliron_op(
    name = "llvm.trunc",
    format = "$0 ` to ` type($0)",
    interfaces = [CastOpInterface, OneResultInterface, OneOpdInterface, NResultsInterface<1>, NOpdsInterface<1>]
)]
pub struct TruncOp;

impl Verify for TruncOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        integer_cast_verify(
            &self.get_operation().deref(ctx),
            ctx,
            ICmpPredicateAttr::ULT,
        )
    }
}

/// Equivalent to LLVM's FPTrunc opcode.
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | Floating-point number |
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | Floating-point number |
#[pliron_op(
    name = "llvm.fptrunc",
    format = "attr($llvm_fast_math_flags, $FastmathFlagsAttr) ` ` $0 ` to ` type($0)",
    interfaces = [CastOpInterface, OneResultInterface, OneOpdInterface, FastMathFlags, NResultsInterface<1>, NOpdsInterface<1>]
)]
pub struct FPTruncOp;

impl Verify for FPTruncOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        // Check operand type to be a float
        let opd_ty = OneOpdInterface::operand_type(self, ctx).deref(ctx);
        let Some(opd_float_ty) = type_cast::<dyn FloatTypeInterface>(&*opd_ty) else {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::OperandTypeErr);
        };
        let res_ty = OneResultInterface::result_type(self, ctx).deref(ctx);
        let Some(res_float_ty) = type_cast::<dyn FloatTypeInterface>(&*res_ty) else {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::ResultTypeErr);
        };

        let opd_size = opd_float_ty.get_semantics().bits;
        let res_size = res_float_ty.get_semantics().bits;
        if opd_size <= res_size {
            return verify_err!(
                self.loc(ctx),
                FloatCastVerifyErr::OperandTypeSmallerThanResult
            );
        }
        Ok(())
    }
}

fn cast_element_types(
    opd_ty: TypeHandle,
    res_ty: TypeHandle,
    ctx: &Context,
    loc: Location,
) -> Result<(TypeHandle, TypeHandle)> {
    let mut opd_elem_ty = opd_ty;
    let mut res_elem_ty = res_ty;
    let mut opd_vec_shape = None;
    let mut res_vec_shape = None;

    if let Some(vec_ty) = opd_ty.deref(ctx).downcast_ref::<VectorType>() {
        opd_elem_ty = vec_ty.elem_type();
        opd_vec_shape = Some((vec_ty.num_elements(), vec_ty.kind()));
    }
    if let Some(vec_ty) = res_ty.deref(ctx).downcast_ref::<VectorType>() {
        res_elem_ty = vec_ty.elem_type();
        res_vec_shape = Some((vec_ty.num_elements(), vec_ty.kind()));
    }

    if opd_vec_shape != res_vec_shape {
        return verify_err!(loc, FloatCastVerifyErr::MismatchedVectorShape);
    }

    Ok((opd_elem_ty, res_elem_ty))
}

/// Equivalent to LLVM's FPToSI opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | Floating-point number |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | Signed integer |
#[pliron_op(
    name = "llvm.fptosi",
    format = "$0 ` to ` type($0)",
    interfaces = [CastOpInterface, OneResultInterface, OneOpdInterface, NResultsInterface<1>, NOpdsInterface<1>]
)]
pub struct FPToSIOp;

impl Verify for FPToSIOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        // Check that the operand is a float and the result is an integer
        let (opd_ty, res_ty) = cast_element_types(
            OneOpdInterface::operand_type(self, ctx),
            OneResultInterface::result_type(self, ctx),
            ctx,
            self.loc(ctx),
        )?;
        let opd_ty = opd_ty.deref(ctx);
        if !type_impls::<dyn FloatTypeInterface>(&*opd_ty) {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::OperandTypeErr);
        };
        let res_ty = res_ty.deref(ctx);
        let Some(res_int_ty) = res_ty.downcast_ref::<IntegerType>() else {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::ResultTypeErr);
        };
        if !res_int_ty.is_signless() {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::ResultTypeErr);
        }
        Ok(())
    }
}

/// Equivalent to LLVM's FPToUI opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | Floating-point number |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | Unsigned integer |
#[pliron_op(
    name = "llvm.fptoui",
    format = "$0 ` to ` type($0)",
    interfaces = [CastOpInterface, OneResultInterface, OneOpdInterface, NResultsInterface<1>, NOpdsInterface<1>]
)]
pub struct FPToUIOp;

impl Verify for FPToUIOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        // Check that the operand is a float and the result is an integer
        let (opd_ty, res_ty) = cast_element_types(
            OneOpdInterface::operand_type(self, ctx),
            OneResultInterface::result_type(self, ctx),
            ctx,
            self.loc(ctx),
        )?;
        let opd_ty = opd_ty.deref(ctx);
        if !type_impls::<dyn FloatTypeInterface>(&*opd_ty) {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::OperandTypeErr);
        };
        let res_ty = res_ty.deref(ctx);
        let Some(res_int_ty) = res_ty.downcast_ref::<IntegerType>() else {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::ResultTypeErr);
        };
        if !res_int_ty.is_signless() {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::ResultTypeErr);
        }
        Ok(())
    }
}

/// Equivalent to LLVM's SIToFP opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | Signed integer |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | Floating-point number |
#[pliron_op(
    name = "llvm.sitofp",
    format = "$0 ` to ` type($0)",
    interfaces = [CastOpInterface, OneResultInterface, OneOpdInterface, NResultsInterface<1>, NOpdsInterface<1>]
)]
pub struct SIToFPOp;

impl Verify for SIToFPOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        // Check that the operand is an integer and the result is a float
        let (opd_ty, res_ty) = cast_element_types(
            OneOpdInterface::operand_type(self, ctx),
            OneResultInterface::result_type(self, ctx),
            ctx,
            self.loc(ctx),
        )?;
        let opd_ty = opd_ty.deref(ctx);
        let Some(opd_ty_int) = opd_ty.downcast_ref::<IntegerType>() else {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::OperandTypeErr);
        };
        if !opd_ty_int.is_signless() {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::OperandTypeErr);
        }
        let res_ty = res_ty.deref(ctx);
        if !type_impls::<dyn FloatTypeInterface>(&*res_ty) {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::ResultTypeErr);
        }
        Ok(())
    }
}

/// Equivalent to LLVM's UIToFP opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `arg` | Unsigned integer |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | Floating-point number |
#[pliron_op(
    name = "llvm.uitofp",
    format = "`<nneg=` attr($llvm_nneg_flag, `pliron::builtin::attributes::BoolAttr`) `> `$0 ` to ` type($0)",
    interfaces = [
        CastOpInterface,
        OneResultInterface,
        OneOpdInterface,
        CastOpWithNNegInterface,
        NNegFlag,
        NResultsInterface<1>,
        NOpdsInterface<1>
    ]
)]
pub struct UIToFPOp;

impl Verify for UIToFPOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        // Check that the operand is an integer and the result is a float
        let (opd_ty, res_ty) = cast_element_types(
            OneOpdInterface::operand_type(self, ctx),
            OneResultInterface::result_type(self, ctx),
            ctx,
            self.loc(ctx),
        )?;
        let opd_ty = opd_ty.deref(ctx);
        let Some(opd_ty_int) = opd_ty.downcast_ref::<IntegerType>() else {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::OperandTypeErr);
        };
        if !opd_ty_int.is_signless() {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::OperandTypeErr);
        }
        let res_ty = res_ty.deref(ctx);
        if !type_impls::<dyn FloatTypeInterface>(&*res_ty) {
            return verify_err!(self.loc(ctx), FloatCastVerifyErr::ResultTypeErr);
        }
        Ok(())
    }
}

/// Equivalent to LLVM's InsertValue opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `aggregate` | LLVM aggregate type |
/// | `value` | LLVM type |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | LLVM aggregate type |
#[pliron_op(
    name = "llvm.insert_value",
    format = "$0 attr($insert_value_indices, $InsertExtractValueIndicesAttr) `, ` $1 ` : ` type($0)",
    interfaces = [OneResultInterface, NResultsInterface<1>, NOpdsInterface<2>],
    attributes = (insert_value_indices: InsertExtractValueIndicesAttr)
)]
pub struct InsertValueOp;

impl InsertValueOp {
    /// Create a new [InsertValueOp].
    /// `aggregate` is the aggregate type and `value` is the value to insert.
    /// `indices` is the list of indices to insert the value at.
    /// The `indices` must be valid for the given `aggregate` type.
    pub fn new(ctx: &mut Context, aggregate: Value, value: Value, indices: Vec<u32>) -> Self {
        use pliron::r#type::Typed;

        let result_type = aggregate.get_type(ctx);
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_type],
            vec![aggregate, value],
            vec![],
            0,
        );
        let op = InsertValueOp { op };
        op.set_attr_insert_value_indices(ctx, InsertExtractValueIndicesAttr(indices));
        op
    }

    /// Get the indices for inserting value into aggregate.
    pub fn indices(&self, ctx: &Context) -> Vec<u32> {
        self.get_attr_insert_value_indices(ctx).unwrap().clone().0
    }
}

impl Verify for InsertValueOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);
        // Ensure that we have the indices as an attribute.
        if self.get_attr_insert_value_indices(ctx).is_none() {
            verify_err!(loc.clone(), InsertExtractValueErr::IndicesAttrErr)?
        }

        use pliron::r#type::Typed;

        // Check that the value we are inserting is of the correct type.
        let aggr_type = self.get_operation().deref(ctx).get_operand(0).get_type(ctx);
        let indices = self.indices(ctx);
        match ExtractValueOp::indexed_type(ctx, aggr_type, &indices) {
            Err(e @ Error { .. }) => {
                // We reset the error type and error origin to be from here
                return Err(Error {
                    kind: ErrorKind::VerificationFailed,
                    backtrace: pliron::std_deps::backtrace::Backtrace::capture(),
                    ..e
                });
            }
            Ok(indexed_type) => {
                if indexed_type != self.get_operation().deref(ctx).get_operand(1).get_type(ctx) {
                    return verify_err!(loc, InsertExtractValueErr::ValueTypeErr);
                }
            }
        }

        Ok(())
    }
}

/// Equivalent to LLVM's ExtractValue opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `aggregate` | LLVM aggregate type |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | LLVM type |
#[pliron_op(
    name = "llvm.extract_value",
    format = "$0 attr($extract_value_indices, $InsertExtractValueIndicesAttr) ` : ` type($0)",
    interfaces = [OneResultInterface, OneOpdInterface, NResultsInterface<1>, NOpdsInterface<1>],
    attributes = (extract_value_indices: InsertExtractValueIndicesAttr)
)]
pub struct ExtractValueOp;

impl Verify for ExtractValueOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);
        // Ensure that we have the indices as an attribute.
        if self.get_attr_extract_value_indices(ctx).is_none() {
            verify_err!(loc.clone(), InsertExtractValueErr::IndicesAttrErr)?
        }

        use pliron::r#type::Typed;
        // Check that the result type matches the indexed type
        let aggr_type = self.get_operation().deref(ctx).get_operand(0).get_type(ctx);
        let indices = self.indices(ctx);
        match Self::indexed_type(ctx, aggr_type, &indices) {
            Err(e @ Error { .. }) => {
                // We reset the error type and error origin to be from here
                return Err(Error {
                    kind: ErrorKind::VerificationFailed,
                    backtrace: pliron::std_deps::backtrace::Backtrace::capture(),
                    ..e
                });
            }
            Ok(indexed_type) => {
                if indexed_type != self.get_operation().deref(ctx).get_type(0) {
                    return verify_err!(loc, InsertExtractValueErr::ValueTypeErr);
                }
            }
        }

        Ok(())
    }
}

impl ExtractValueOp {
    /// Create a new [ExtractValueOp].
    /// `aggregate` is the aggregate type and `indices` is the list of indices to extract the value from.
    /// The `indices` must be valid for the given `aggregate` type.
    /// The result type of the operation is the type of the value at the given indices.
    pub fn new(ctx: &mut Context, aggregate: Value, indices: Vec<u32>) -> Result<Self> {
        use pliron::r#type::Typed;
        let result_type = Self::indexed_type(ctx, aggregate.get_type(ctx), &indices)?;
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_type],
            vec![aggregate],
            vec![],
            0,
        );
        let op = ExtractValueOp { op };
        op.set_attr_extract_value_indices(ctx, InsertExtractValueIndicesAttr(indices));
        Ok(op)
    }

    /// Get the indices for extracting value from aggregate.
    pub fn indices(&self, ctx: &Context) -> Vec<u32> {
        self.get_attr_extract_value_indices(ctx).unwrap().clone().0
    }

    /// Returns the type of the value at the given indices in the given aggregate type.
    pub fn indexed_type(
        ctx: &Context,
        aggr_type: TypeHandle,
        indices: &[u32],
    ) -> Result<TypeHandle> {
        fn indexed_type_inner(
            ctx: &Context,
            aggr_type: TypeHandle,
            mut idx_itr: impl Iterator<Item = u32>,
        ) -> Result<TypeHandle> {
            let Some(idx) = idx_itr.next() else {
                return Ok(aggr_type);
            };
            let aggr_type = &*aggr_type.deref(ctx);
            if let Some(st) = aggr_type.downcast_ref::<StructType>() {
                if st.is_opaque() || idx as usize >= st.num_fields() {
                    return arg_err_noloc!(InsertExtractValueErr::InvalidIndicesErr);
                }
                indexed_type_inner(ctx, st.field_type(idx as usize), idx_itr)
            } else if let Some(at) = aggr_type.downcast_ref::<ArrayType>() {
                if idx as u64 >= at.size() {
                    return arg_err_noloc!(InsertExtractValueErr::InvalidIndicesErr);
                }
                indexed_type_inner(ctx, at.elem_type(), idx_itr)
            } else {
                arg_err_noloc!(InsertExtractValueErr::InvalidIndicesErr)
            }
        }
        indexed_type_inner(ctx, aggr_type, indices.iter().cloned())
    }
}

#[derive(Error, Debug)]
pub enum InsertExtractValueErr {
    #[error("Insert/Extract value instruction has no or incorrect indices attribute")]
    IndicesAttrErr,
    #[error("Invalid indices on insert/extract value instruction")]
    InvalidIndicesErr,
    #[error("Value being inserted / extracted does not match the type of the indexed aggregate")]
    ValueTypeErr,
}

/// Equivalent to LLVM's InsertElement opcode.
///
//// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `vector` | LLVM vector type |
/// | `element` | LLVM type |
/// | `index` | u32 |
///
/// /// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | LLVM vector type |
#[pliron_op(
    name = "llvm.insert_element",
    format = "$0 `, ` $1 `, ` $2 ` : ` type($0)",
    interfaces = [OneResultInterface, NResultsInterface<1>, NOpdsInterface<3>],
    operands = (vector, element, index)
)]
pub struct InsertElementOp;
impl Verify for InsertElementOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        use pliron::r#type::Typed;

        let loc = self.loc(ctx);
        let op = &*self.op.deref(ctx);
        let vector_ty = op.get_operand(0).get_type(ctx);
        let element_ty = op.get_operand(1).get_type(ctx);
        let index_ty = op.get_operand(2).get_type(ctx);

        let vector_ty = vector_ty.deref(ctx);
        let vector_ty = vector_ty.downcast_ref::<VectorType>();
        if vector_ty.is_none_or(|ty| ty.elem_type() != element_ty) {
            return verify_err!(loc, InsertExtractElementOpVerifyErr::ElementTypeErr);
        }

        if !index_ty.deref(ctx).is::<IntegerType>() {
            return verify_err!(loc, InsertExtractElementOpVerifyErr::IndexTypeErr);
        }

        Ok(())
    }
}

impl InsertElementOp {
    /// Create a new [InsertElementOp].
    pub fn new(ctx: &mut Context, vector: Value, element: Value, index: Value) -> Self {
        use pliron::r#type::Typed;

        let result_type = vector.get_type(ctx);
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_type],
            vec![vector, element, index],
            vec![],
            0,
        );
        InsertElementOp { op }
    }

    /// Get the vector type of the InsertElementOp.
    pub fn vector_type(&self, ctx: &Context) -> TypedHandle<VectorType> {
        let ty = self.get_operation().deref(ctx).get_type(0);
        TypedHandle::<VectorType>::from_handle(ty, ctx)
            .expect("InsertElementOp result type is not a VectorType")
    }
}

#[derive(Error, Debug)]
pub enum InsertExtractElementOpVerifyErr {
    #[error("Element type must match vector element type")]
    ElementTypeErr,
    #[error("Index type must be signless integer")]
    IndexTypeErr,
}

/// ExtractElementOp
/// Equivalent to LLVM's ExtractElement opcode.
/// /// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `vector` | LLVM vector type |
/// | `index` | u32 |
/// /// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | LLVM type |
#[pliron_op(
    name = "llvm.extract_element",
    format = "$0 `, ` $1 ` : ` type($0)",
    interfaces = [OneResultInterface, NResultsInterface<1>, NOpdsInterface<2>],
    operands = (vector, index)
)]
pub struct ExtractElementOp;

impl Verify for ExtractElementOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        use pliron::r#type::Typed;
        let loc = self.loc(ctx);
        let op = &*self.op.deref(ctx);
        let vector_ty = op.get_operand(0).get_type(ctx);
        let index_ty = op.get_operand(1).get_type(ctx);
        let vector_ty = vector_ty.deref(ctx);
        let vector_ty = vector_ty.downcast_ref::<VectorType>();
        if vector_ty.is_none_or(|ty| ty.elem_type() != op.get_type(0)) {
            return verify_err!(loc, InsertExtractElementOpVerifyErr::ElementTypeErr);
        }
        if !index_ty.deref(ctx).is::<IntegerType>() {
            return verify_err!(loc, InsertExtractElementOpVerifyErr::IndexTypeErr);
        }
        Ok(())
    }
}

impl ExtractElementOp {
    /// Create a new [ExtractElementOp].
    pub fn new(ctx: &mut Context, vector: Value, index: Value) -> Self {
        use pliron::r#type::Typed;

        let result_type = vector
            .get_type(ctx)
            .deref(ctx)
            .downcast_ref::<VectorType>()
            .expect("ExtractElementOp vector operand must be a vector type")
            .elem_type();

        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_type],
            vec![vector, index],
            vec![],
            0,
        );
        ExtractElementOp { op }
    }

    /// Get the vector type of the ExtractElementOp.
    pub fn vector_type(&self, ctx: &Context) -> TypedHandle<VectorType> {
        use pliron::r#type::Typed;
        let ty = self.get_operand_vector(ctx).get_type(ctx);
        TypedHandle::<VectorType>::from_handle(ty, ctx)
            .expect("ExtractElementOp vector operand type is not a VectorType")
    }
}

/// ShuffleVectorOp
/// Equivalent to LLVM's ShuffleVector opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `vector1` | LLVM vector type |
/// | `vector2` | LLVM vector type |
/// | `mask` | LLVM vector type |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | LLVM vector type |
#[pliron_op(
    name = "llvm.shuffle_vector",
    format = "$0 `, ` $1 `, ` attr($llvm_shuffle_vector_mask, $ShuffleVectorMaskAttr) ` : ` type($0)",
    interfaces = [OneResultInterface, NResultsInterface<1>, NOpdsInterface<2>],
    attributes = (llvm_shuffle_vector_mask: ShuffleVectorMaskAttr)
)]
pub struct ShuffleVectorOp;
impl Verify for ShuffleVectorOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        use pliron::r#type::Typed;

        let loc = self.loc(ctx);
        let op = &*self.op.deref(ctx);
        let vector1_ty = op.get_operand(0).get_type(ctx);
        let vector2_ty = op.get_operand(1).get_type(ctx);

        let vector1_ty = vector1_ty.deref(ctx);
        let vector1_ty = vector1_ty.downcast_ref::<VectorType>();
        let vector2_ty = vector2_ty.deref(ctx);
        let vector2_ty = vector2_ty.downcast_ref::<VectorType>();

        let (Some(v1_ty), Some(v2_ty)) = (vector1_ty, vector2_ty) else {
            return verify_err!(loc, ShuffleVectorOpVerifyErr::OperandsTypeErr);
        };

        if v1_ty != v2_ty {
            return verify_err!(loc, ShuffleVectorOpVerifyErr::OperandsTypeErr);
        }

        let res_ty = op.get_type(0).deref(ctx);
        let res_ty = res_ty.downcast_ref::<VectorType>();
        let Some(res_ty) = res_ty else {
            return verify_err!(loc, ShuffleVectorOpVerifyErr::ResultTypeErr);
        };

        if res_ty.elem_type() != v1_ty.elem_type()
            || res_ty.num_elements() as usize
                != self.get_attr_llvm_shuffle_vector_mask(ctx).unwrap().0.len()
        {
            return verify_err!(loc, ShuffleVectorOpVerifyErr::ResultTypeErr);
        }

        Ok(())
    }
}

/// The undef mask element used in ShuffleVectorOp masks.
#[cfg(feature = "llvm-sys")]
pub static SHUFFLE_VECTOR_UNDEF_MASK_ELEM: std::sync::LazyLock<i32> =
    std::sync::LazyLock::new(llvm_get_undef_mask_elem);
#[cfg(not(feature = "llvm-sys"))]
pub static SHUFFLE_VECTOR_UNDEF_MASK_ELEM: i32 = -1;

impl ShuffleVectorOp {
    /// Create a new [ShuffleVectorOp].
    pub fn new(ctx: &mut Context, vector1: Value, vector2: Value, mask: Vec<i32>) -> Self {
        use pliron::r#type::Typed;

        let (elem_ty, kind) = {
            let vector1_ty = vector1.get_type(ctx).deref(ctx);
            let opd_vec_ty = vector1_ty
                .downcast_ref::<VectorType>()
                .expect("ShuffleVectorOp vector1 operand must be a vector type");
            (opd_vec_ty.elem_type(), opd_vec_ty.kind())
        };

        let result_type = VectorType::get(
            ctx,
            elem_ty,
            mask.len()
                .try_into()
                .expect("ShuffleVectorOp mask length too large"),
            kind,
        );
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_type.into()],
            vec![vector1, vector2],
            vec![],
            0,
        );

        let mask_attr = ShuffleVectorMaskAttr(mask);
        let op = ShuffleVectorOp { op };
        op.set_attr_llvm_shuffle_vector_mask(ctx, mask_attr);
        op
    }
}

#[derive(Error, Debug)]
pub enum ShuffleVectorOpVerifyErr {
    #[error("Both operands must be equivalent vector types")]
    OperandsTypeErr,
    #[error("Result type must be a vector type with correct element type and size")]
    ResultTypeErr,
}

/// Equivalent to LLVM's Select opcode.
///
/// ### Operands
/// | operand | description |
/// |-----|-------|
/// | `condition` | i1 |
/// | `true_dest` | any type |
/// | `false_dest` | any type |
///
/// ### Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | any type |
#[pliron_op(
    name = "llvm.select",
    format = "$0 ` ? ` $1 ` : ` $2 ` : ` type($0)",
    interfaces = [OneResultInterface, NResultsInterface<1>, NOpdsInterface<3>]
)]
pub struct SelectOp;

impl SelectOp {
    /// Create a new [SelectOp].
    pub fn new(ctx: &mut Context, cond: Value, true_val: Value, false_val: Value) -> Self {
        use pliron::r#type::Typed;

        let result_type = true_val.get_type(ctx);
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_type],
            vec![cond, true_val, false_val],
            vec![],
            0,
        );
        SelectOp { op }
    }
}

impl Verify for SelectOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        use pliron::r#type::Typed;

        let loc = self.loc(ctx);
        let op = &*self.op.deref(ctx);
        let ty = op.get_type(0);
        let cond_ty = op.get_operand(0).get_type(ctx);
        let true_ty = op.get_operand(1).get_type(ctx);
        let false_ty = op.get_operand(2).get_type(ctx);
        if ty != true_ty || ty != false_ty {
            return verify_err!(loc, SelectOpVerifyErr::ResultTypeErr);
        }

        let mut cond_ty = cond_ty.deref(ctx);
        if let Some(vec_ty) = cond_ty.downcast_ref::<VectorType>() {
            if let Some(opd_vec_ty) = ty.deref(ctx).downcast_ref::<VectorType>()
                && vec_ty.num_elements() == opd_vec_ty.num_elements()
            {
                // We're good, both the condition and operand are vectors of the same length
            } else {
                return verify_err!(loc, SelectOpVerifyErr::ConditionTypeErr);
            }
            cond_ty = vec_ty.elem_type().deref(ctx);
        }

        let cond_ty = cond_ty.downcast_ref::<IntegerType>();
        if cond_ty.is_none_or(|ty| ty.width() != 1) {
            return verify_err!(loc, SelectOpVerifyErr::ConditionTypeErr);
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum SelectOpVerifyErr {
    #[error("Result must be the same as the true and false destination types")]
    ResultTypeErr,
    #[error("Condition must be an i1 or a vector of i1 equal in length to the operand vectors")]
    ConditionTypeErr,
}

/// Floating-point negation
/// Equivalent to LLVM's `fneg` instruction.
///
/// Operands:
/// | operand | description |
/// |-----|-------|
/// | `arg` | float |
///
/// Result(s):
/// | result | description |
/// |-----|-------|
/// | `res` | float |
#[pliron_op(
    name = "llvm.fneg",
    format = "attr($llvm_fast_math_flags, $FastmathFlagsAttr) ` ` $0 ` : ` type($0)",
    interfaces = [
        OneResultInterface,
        OneOpdInterface,
        SameResultsType,
        SameOperandsType,
        SameOperandsAndResultType,
        FastMathFlags,
        NResultsInterface<1>,
        NOpdsInterface<1>,
        AtLeastNOpdsInterface<1>,
        AtLeastNResultsInterface<1>
    ]
)]
pub struct FNegOp;

impl Verify for FNegOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        use pliron::r#type::Typed;

        let loc = self.loc(ctx);
        let op = &*self.op.deref(ctx);
        let arg_ty = op.get_operand(0).get_type(ctx);
        if !type_impls::<dyn FloatTypeInterface>(&*arg_ty.deref(ctx)) {
            return verify_err!(loc, FNegOpVerifyErr::ArgumentMustBeFloat);
        }
        Ok(())
    }
}

impl FNegOp {
    /// Create a new [FNegOp].
    pub fn new_with_fast_math_flags(
        ctx: &mut Context,
        arg: Value,
        fast_math_flags: FastmathFlagsAttr,
    ) -> Self {
        use pliron::r#type::Typed;
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![arg.get_type(ctx)],
            vec![arg],
            vec![],
            0,
        );
        let op = FNegOp { op };
        op.set_fast_math_flags(ctx, fast_math_flags);
        op
    }
}

#[derive(Error, Debug)]
pub enum FNegOpVerifyErr {
    #[error("Argument must be a float")]
    ArgumentMustBeFloat,
    #[error("Fast math flags must be set")]
    FastMathFlagsMustBeSet,
}

macro_rules! new_float_bin_op {
    (   $(#[$outer:meta])*
        $op_name:ident, $op_id:literal
    ) => {
        $(#[$outer])*
        /// ### Operands:
        ///
        /// | operand | description |
        /// |-----|-------|
        /// | `lhs` | float |
        /// | `rhs` | float |
        ///
        /// ### Result(s):
        ///
        /// | result | description |
        /// |-----|-------|
        /// | `res` | float |
        #[pliron_op(
            name = $op_id,
            format = "attr($llvm_fast_math_flags, $FastmathFlagsAttr) ` ` $0 `, ` $1 ` : ` type($0)",
            interfaces = [
                OneResultInterface, SameOperandsType, SameResultsType,
                AtLeastNOpdsInterface<1>, AtLeastNResultsInterface<1>,
                SameOperandsAndResultType, BinArithOp, FloatBinArithOp,
                FloatBinArithOpWithFastMathFlags, FastMathFlags, NResultsInterface<1>, NOpdsInterface<2>
            ],
            verifier = "succ"
        )]
        pub struct $op_name;
    }
}

new_float_bin_op! {
    /// Equivalent to LLVM's `fadd` instruction.
    FAddOp,
    "llvm.fadd"
}

new_float_bin_op! {
    /// Equivalent to LLVM's `fsub` instruction.
    FSubOp,
    "llvm.fsub"
}

new_float_bin_op! {
    /// Equivalent to LLVM's `fmul` instruction.
    FMulOp,
    "llvm.fmul"
}

new_float_bin_op! {
    /// Equivalent to LLVM's `fdiv` instruction.
    FDivOp,
    "llvm.fdiv"
}

new_float_bin_op! {
    /// Equivalent to LLVM's `frem` instruction.
    FRemOp,
    "llvm.frem"
}

/// Equivalent to LLVM'same `fcmp` instruction.
///
/// ### Operand(s):
/// | operand | description |
/// |-----|-------|
/// | `lhs` | float |
/// | `rhs` | float |
///
/// ### Result(s):
///
/// | result | description |
/// |-----|-------|
/// | `res` | 1-bit signless integer |
#[pliron_op(
    name = "llvm.fcmp",
    format = "attr($llvm_fast_math_flags, $FastmathFlagsAttr) ` ` $0 ` <` attr($fcmp_predicate, $FCmpPredicateAttr) `> ` $1 ` : ` type($0)",
    interfaces = [
        OneResultInterface,
        SameOperandsType,
        AtLeastNOpdsInterface<1>,
        FastMathFlags,
        NResultsInterface<1>,
        NOpdsInterface<2>
    ],
    attributes = (fcmp_predicate: FCmpPredicateAttr)
)]
pub struct FCmpOp;

impl FCmpOp {
    /// Create a new [FCmpOp]
    pub fn new(ctx: &mut Context, pred: FCmpPredicateAttr, lhs: Value, rhs: Value) -> Self {
        let bool_ty = IntegerType::get(ctx, 1, Signedness::Signless);
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![bool_ty.into()],
            vec![lhs, rhs],
            vec![],
            0,
        );
        let op = FCmpOp { op };
        op.set_attr_fcmp_predicate(ctx, pred);
        op
    }

    /// Get the predicate
    pub fn predicate(&self, ctx: &Context) -> FCmpPredicateAttr {
        self.get_attr_fcmp_predicate(ctx)
            .expect("FCmpOp missing or incorrect predicate attribute type")
            .clone()
    }
}

impl Verify for FCmpOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);

        if self.get_attr_fcmp_predicate(ctx).is_none() {
            verify_err!(loc.clone(), FCmpOpVerifyErr::PredAttrErr)?
        }

        let res_ty: TypedHandle<IntegerType> = TypedHandle::from_handle(self.result_type(ctx), ctx)
            .map_err(|mut err| {
                err.set_loc(loc.clone());
                err
            })?;

        if res_ty.deref(ctx).width() != 1 {
            return verify_err!(loc, FCmpOpVerifyErr::ResultNotBool);
        }

        let opd_ty = self.operand_type(ctx).deref(ctx);
        if !(type_impls::<dyn FloatTypeInterface>(&*opd_ty)) {
            return verify_err!(loc, FCmpOpVerifyErr::IncorrectOperandsType);
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum FCmpOpVerifyErr {
    #[error("Result must be 1-bit integer (bool)")]
    ResultNotBool,
    #[error("Operand must be floating point type")]
    IncorrectOperandsType,
    #[error("Missing or incorrect predicate attribute")]
    PredAttrErr,
}

/// All LLVM intrinsic calls are represented by this [Op].
/// Same as MLIR's [llvm.call_intrinsic](https://mlir.llvm.org/docs/Dialects/LLVM/#llvmcall_intrinsic-llvmcallintrinsicop).
#[pliron_op(
    name = "llvm.call_intrinsic",
    interfaces = [OneResultInterface, NResultsInterface<1>],
    attributes = (
        llvm_intrinsic_name: StringAttr,
        llvm_intrinsic_type: TypeAttr,
        llvm_intrinsic_fastmath_flags: FastmathFlagsAttr
    )
)]
pub struct CallIntrinsicOp;

impl CallIntrinsicOp {
    /// Create a new [CallIntrinsicOp].
    pub fn new(
        ctx: &mut Context,
        intrinsic_name: StringAttr,
        intrinsic_type: TypedHandle<FuncType>,
        operands: Vec<Value>,
    ) -> Self {
        let res_ty = intrinsic_type.deref(ctx).result_type();
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![res_ty],
            operands,
            vec![],
            0,
        );
        let op = CallIntrinsicOp { op };
        op.set_attr_llvm_intrinsic_name(ctx, intrinsic_name);
        op.set_attr_llvm_intrinsic_type(ctx, TypeAttr::new(intrinsic_type.into()));
        op
    }
}

impl Printable for CallIntrinsicOp {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        // [result = ] llvm.call_intrinsic @name <FastMathFlags> (operands) : type
        if let Some(res) = self.op.deref(ctx).results().next() {
            write!(f, "{} = ", res.disp(ctx))?;
        }

        write!(
            f,
            "{} @{} ",
            Self::get_opid_static(),
            self.get_attr_llvm_intrinsic_name(ctx)
                .expect("CallIntrinsicOp missing or incorrect intrinsic name attribute")
                .disp(ctx),
        )?;

        if let Some(fmf) = self.get_attr_llvm_intrinsic_fastmath_flags(ctx)
            && *fmf != FastmathFlagsAttr::default()
        {
            write!(f, " {} ", fmf.disp(ctx))?;
        }

        write!(
            f,
            "({}) : {}",
            iter_with_sep(
                self.op.deref(ctx).operands(),
                printable::ListSeparator::CharSpace(',')
            )
            .disp(ctx),
            self.get_attr_llvm_intrinsic_type(ctx)
                .expect("CallIntrinsicOp missing or incorrect intrinsic type attribute")
                .disp(ctx),
        )
    }
}

impl Parsable for CallIntrinsicOp {
    type Arg = Vec<(Identifier, Location)>;
    type Parsed = OpObj;
    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        results: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let pos = state_stream.loc();

        let mut parser = (
            spaced(token('@').with(StringAttr::parser(()))),
            optional(spaced(FastmathFlagsAttr::parser(()))),
            delimited_list_parser('(', ')', ',', ssa_opd_parser()).skip(spaced(token(':'))),
            spaced(type_parser()),
        );

        // Parse and build the call intrinsic op.
        let (iname, fmf, operands, ftype) = parser.parse_stream(state_stream).into_result()?.0;

        let ctx = &mut state_stream.state.ctx;
        let intr_ty = TypedHandle::<FuncType>::from_handle(ftype, ctx).map_err(|mut err| {
            err.set_loc(pos);
            err
        })?;
        let op = CallIntrinsicOp::new(ctx, iname, intr_ty, operands);
        if let Some(fmf) = fmf {
            op.set_attr_llvm_intrinsic_fastmath_flags(ctx, fmf);
        }
        process_parsed_ssa_defs(state_stream, &results, op.get_operation())?;
        Ok(OpObj::new(op)).into_parse_result()
    }
}

#[derive(Error, Debug)]
pub enum CallIntrinsicVerifyErr {
    #[error("Missing or incorrect intrinsic name attribute")]
    MissingIntrinsicNameAttr,
    #[error("Missing or incorrect intrinsic type attribute")]
    MissingIntrinsicTypeAttr,
    #[error("Number or types of operands does not match intrinsic type")]
    OperandsMismatch,
    #[error("Number or types of results does not match intrinsic type")]
    ResultsMismatch,
    #[error("Intrinsic name does not correspond to a known LLVM intrinsic")]
    UnknownIntrinsicName,
}

impl Verify for CallIntrinsicOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        // Check that the intrinsic name and type attributes are present.
        let Some(name) = self.get_attr_llvm_intrinsic_name(ctx) else {
            return verify_err!(
                self.loc(ctx),
                CallIntrinsicVerifyErr::MissingIntrinsicNameAttr
            );
        };

        let Some(ty) = self
            .get_attr_llvm_intrinsic_type(ctx)
            .and_then(|ty| TypedHandle::<FuncType>::from_handle(ty.get_type(ctx), ctx).ok())
        else {
            return verify_err!(
                self.loc(ctx),
                CallIntrinsicVerifyErr::MissingIntrinsicTypeAttr
            );
        };

        let arg_types = ty.deref(ctx).arg_types();
        let res_type = ty.deref(ctx).result_type();

        // Check that the operand and result types match the intrinsic type.
        let op = &*self.op.deref(ctx);
        let intrinsic_arg_types = ty.deref(ctx).arg_types();
        if op.operands().count() != intrinsic_arg_types.len() {
            return verify_err!(self.loc(ctx), CallIntrinsicVerifyErr::OperandsMismatch);
        }

        for (i, operand) in op.operands().enumerate() {
            let opd_ty = pliron::r#type::Typed::get_type(&operand, ctx);
            if opd_ty != arg_types[i] {
                return verify_err!(self.loc(ctx), CallIntrinsicVerifyErr::OperandsMismatch);
            }
        }

        let mut result_types = op.result_types();
        if let Some(result_type) = result_types.next()
            && result_type == res_type
            && result_types.next().is_none()
        {
        } else {
            return verify_err!(self.loc(ctx), CallIntrinsicVerifyErr::ResultsMismatch);
        }

        let name: String = name.clone().into();
        #[cfg(feature = "llvm-sys")]
        if llvm_lookup_intrinsic_id(&name).is_none() {
            return verify_err!(self.loc(ctx), CallIntrinsicVerifyErr::UnknownIntrinsicName);
        }
        #[cfg(not(feature = "llvm-sys"))]
        // We can't verify the intrinsic name without llvm-sys, so just check that it's not empty.
        if name.is_empty() {
            return verify_err!(self.loc(ctx), CallIntrinsicVerifyErr::UnknownIntrinsicName);
        }

        Ok(())
    }
}

/// Equivalent to LLVM's `va_arg` operation.
#[pliron_op(
    name = "llvm.va_arg",
    format = "$0 ` : ` type($0)",
    interfaces = [OneResultInterface, OneOpdInterface, NResultsInterface<1>, NOpdsInterface<1>]
)]
pub struct VAArgOp;

#[derive(Error, Debug)]
pub enum VAArgOpVerifyErr {
    #[error("Operand must be a pointer type")]
    OperandNotPointer,
}

impl Verify for VAArgOp {
    fn verify(&self, ctx: &Context) -> Result<()> {
        let loc = self.loc(ctx);

        // Check that the argument is a pointer.
        let opd_ty = self.operand_type(ctx).deref(ctx);
        if !opd_ty.is::<PointerType>() {
            return verify_err!(loc, VAArgOpVerifyErr::OperandNotPointer);
        }

        Ok(())
    }
}

impl VAArgOp {
    /// Create a new [VAArgOp].
    pub fn new(ctx: &mut Context, list: Value, ty: TypeHandle) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![ty],
            vec![list],
            vec![],
            0,
        );
        VAArgOp { op }
    }
}

/// Equivalent to LLVM's `func` operation.
/// See [llvm.func](https://mlir.llvm.org/docs/Dialects/LLVM/#llvmfunc-llvmllvmfuncop).
#[pliron_op(
    name = "llvm.func",
    interfaces = [
        SymbolOpInterface,
        IsolatedFromAboveInterface,
        AtMostNRegionsInterface<1>,
        AtMostOneRegionInterface,
        NResultsInterface<0>,
        NOpdsInterface<0>,
        LlvmSymbolName
    ],
    attributes = (llvm_func_type: TypeAttr, llvm_function_linkage: LinkageAttr)
)]
pub struct FuncOp;

impl FuncOp {
    /// Create a new empty [FuncOp].
    pub fn new(ctx: &mut Context, name: Identifier, ty: TypedHandle<FuncType>) -> Self {
        let ty_attr = TypeAttr::new(ty.into());
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        let opop = FuncOp { op };
        opop.set_symbol_name(ctx, name);
        opop.set_attr_llvm_func_type(ctx, ty_attr);

        opop
    }

    /// Get the function signature (type).
    pub fn get_type(&self, ctx: &Context) -> TypedHandle<FuncType> {
        let ty = attr_cast::<dyn TypedAttrInterface>(&*self.get_attr_llvm_func_type(ctx).unwrap())
            .unwrap()
            .get_type(ctx);
        TypedHandle::from_handle(ty, ctx).unwrap()
    }

    /// Get the entry block (if it exists) of this function.
    pub fn get_entry_block(&self, ctx: &Context) -> Option<Ptr<BasicBlock>> {
        self.op
            .deref(ctx)
            .regions()
            .next()
            .and_then(|region| region.deref(ctx).get_head())
    }

    /// Get the entry block of this function, creating it if it does not exist.
    pub fn get_or_create_entry_block(&self, ctx: &mut Context) -> Ptr<BasicBlock> {
        if let Some(entry_block) = self.get_entry_block(ctx) {
            return entry_block;
        }

        // Create an empty entry block.
        assert!(
            self.op.deref(ctx).regions().next().is_none(),
            "FuncOp already has a region, but no block inside it"
        );
        let region = Operation::add_region(self.op, ctx);
        let arg_types = self.get_type(ctx).deref(ctx).arg_types().clone();
        let body = BasicBlock::new(ctx, Some("entry".try_into().unwrap()), arg_types);
        body.insert_at_front(region, ctx);
        body
    }
}

impl pliron::r#type::Typed for FuncOp {
    fn get_type(&self, ctx: &Context) -> TypeHandle {
        self.get_type(ctx).into()
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

        // Print attributes except for function type and symbol name.
        let mut attributes_to_print_separately =
            self.op.deref(ctx).attributes.clone_skip_outlined();
        attributes_to_print_separately
            .0
            .retain(|key, _| key != &*ATTR_KEY_LLVM_FUNC_TYPE && key != &*ATTR_KEY_SYM_NAME);
        indented_block!(state, {
            write!(
                f,
                "{}{}",
                indented_nl(state),
                attributes_to_print_separately.disp(ctx)
            )?;
        });

        if let Some(r) = self.get_region(ctx) {
            write!(f, " ")?;
            r.fmt(ctx, state, f)?;
        }
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
            spaced(AttributeDict::parser(())),
            spaced(optional(Region::parser(op))),
        );

        // Parse and build the function, providing name and type details.
        parser
            .parse_stream(state_stream)
            .map(|(fname, fty, attrs, _region)| -> OpObj {
                let ctx = &mut state_stream.state.ctx;
                op.deref_mut(ctx).attributes = attrs;
                let ty_attr = TypeAttr::new(fty);
                let opop = FuncOp { op };
                opop.set_symbol_name(ctx, fname);
                opop.set_attr_llvm_func_type(ctx, ty_attr);
                OpObj::new(opop)
            })
            .into()
    }
}

#[derive(Error, Debug)]
#[error("llvm.func op does not have llvm.func type")]
pub struct FuncOpTypeErr;

impl Verify for FuncOp {
    fn verify(&self, _ctx: &Context) -> Result<()> {
        Ok(())
    }
}

impl IsDeclaration for FuncOp {
    fn is_declaration(&self, ctx: &Context) -> bool {
        self.get_region(ctx).is_none()
    }
}
