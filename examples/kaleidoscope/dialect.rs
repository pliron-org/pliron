//! Kaleidoscope dialect operation definitions used across tutorial chapters.

use awint::bw;
use pliron::{
    builtin::{
        attributes::{IdentifierAttr, IntegerAttr, TypeAttr},
        op_interfaces::{
            AtLeastNOpdsInterface, AtLeastNResultsInterface, IsTerminatorInterface, NOpdsInterface,
            NRegionsInterface, NResultsInterface, OneResultInterface, OperandNOfType,
            ResultNOfType, SameOperandsAndResultType, SameOperandsType, SameResultsType,
            SingleBlockRegionInterface,
        },
        types::{IntegerType, Signedness},
    },
    common_traits::Verify,
    context::{Context, Ptr},
    derive::{pliron_attr, pliron_op},
    op::Op,
    operation::Operation,
    region::Region,
    result::Result,
    r#type::{TypeObj, Typed},
    utils::apint::APInt,
    value::Value,
    verify_err,
};
use pliron_llvm::types::PointerType;

/// Materializes literal constants from AST expressions like `Expr::Integer`.
// ANCHOR: constant_op_decl
#[pliron_op(
    name = "kaleidoscope.constant",
    format = "attr($value, $IntegerAttr) ` : ` type($0)",
    interfaces = [NOpdsInterface<0>, OneResultInterface, NResultsInterface<1>],
    attributes = (value: IntegerAttr),
    verifier = "succ",
)]
pub struct ConstantOp;
// ANCHOR_END: constant_op_decl

// ANCHOR: constant_op_new
impl ConstantOp {
    pub fn new_i64(ctx: &mut Context, value: i64) -> Self {
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);
        let value_attr = IntegerAttr::new(i64_ty, APInt::from_i64(value, bw(64)));
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![i64_ty.into()],
            vec![],
            vec![],
            0,
        );
        let op = ConstantOp { op };
        op.set_attr_value(ctx, value_attr);
        op
    }

    pub fn value_attr(&self, ctx: &Context) -> IntegerAttr {
        self.get_attr_value(ctx)
            .expect("ConstantOp must carry a value attribute")
            .clone()
    }
}
// ANCHOR_END: constant_op_new

// ANCHOR: binop_kind_attr
/// Encodes which AST binary operator a `kaleidoscope.binop` represents.
#[pliron_attr(name = "kaleidoscope.binop_kind", format, verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Copy, Debug, Hash)]
pub enum BinOpKind {
    Add,
    Sub,
    Mul,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}
// ANCHOR_END: binop_kind_attr

/// Declares mutable storage for AST declarations (`var name` / `var name = ...`).
///
/// The op result is always an LLVM pointer slot, and the declared value type is
/// carried in the `var_type` attribute.
// ANCHOR: decl_op_decl
#[pliron_op(
    name = "kaleidoscope.decl",
    format = "attr($var_type, $TypeAttr) ` : ` type($0)",
    interfaces = [
        NOpdsInterface<0>,
        OneResultInterface,
        NResultsInterface<1>,
        ResultNOfType<0, PointerType>
    ],
    attributes = (var_type: TypeAttr),
    verifier = "succ",
)]
pub struct DeclOp;
// ANCHOR_END: decl_op_decl

impl DeclOp {
    /// Creates a new `DeclOp` with the specified variable type.
    /// The op result is always a pointer slot, and the variable type
    /// is stored in the `var_type` attribute.
    pub fn new(ctx: &mut Context, var_ty: Ptr<TypeObj>) -> Self {
        let ptr_ty = PointerType::get(ctx, 0).into();
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![ptr_ty],
            vec![],
            vec![],
            0,
        );
        let op = DeclOp { op };
        op.set_attr_var_type(ctx, TypeAttr::new(var_ty));
        op
    }

    pub fn variable_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        self.get_attr_var_type(ctx)
            .expect("DeclOp must carry var_type")
            .get_type(ctx)
    }
}

/// Reads from a declared variable slot when the AST references a variable.
// ANCHOR: load_op_decl
#[pliron_op(
    name = "kaleidoscope.load",
    format = "$0",
    interfaces = [
        NOpdsInterface<1>,
        OneResultInterface,
        NResultsInterface<1>
    ],
    verifier = "succ",
)]
pub struct LoadOp;
// ANCHOR_END: load_op_decl

impl LoadOp {
    pub fn new(ctx: &mut Context, slot: Value, result_ty: Ptr<TypeObj>) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_ty],
            vec![slot],
            vec![],
            0,
        );
        LoadOp { op }
    }

    pub fn slot(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(0)
    }
}

/// Writes to a declared variable slot for AST assignments (`name = expr`).
// ANCHOR: store_op_decl
#[pliron_op(
    name = "kaleidoscope.store",
    format = "`*` $0 ` <- ` $1",
    interfaces = [NOpdsInterface<2>, NResultsInterface<0>],
    verifier = "succ",
)]
pub struct StoreOp;
// ANCHOR_END: store_op_decl

impl StoreOp {
    pub fn new(ctx: &mut Context, slot: Value, value: Value) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![],
            vec![slot, value],
            vec![],
            0,
        );
        StoreOp { op }
    }

    pub fn slot(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(0)
    }

    pub fn stored_value(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(1)
    }
}

/// Lowers all AST binary expressions into one op with a kind attribute.
// ANCHOR: binop_decl
#[pliron_op(
    name = "kaleidoscope.binop",
    format = "$0 ` `attr($kind, $BinOpKind) ` ` $1 ` : ` type($0)",
    interfaces = [
        AtLeastNOpdsInterface<1>,
        AtLeastNResultsInterface<1>,
        NOpdsInterface<2>,
        OneResultInterface,
        NResultsInterface<1>,
        SameOperandsType,
        SameResultsType,
        SameOperandsAndResultType
    ],
    attributes = (kind: BinOpKind),
    verifier = "succ"
)]
pub struct BinOp;
// ANCHOR_END: binop_decl

// ANCHOR: binop_methods
impl BinOp {
    /// Creates a new `BinOp` of the specified kind with the given operands.
    pub fn new(ctx: &mut Context, kind: BinOpKind, lhs: Value, rhs: Value) -> Self {
        let result_ty = lhs.get_type(ctx);
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_ty],
            vec![lhs, rhs],
            vec![],
            0,
        );
        let op = BinOp { op };
        op.set_attr_kind(ctx, kind);
        op
    }

    /// Returns the `BinOpKind` of this `BinOp`.
    pub fn kind(&self, ctx: &Context) -> BinOpKind {
        *self
            .get_attr_kind(ctx)
            .expect("BinOp must carry kind attribute")
    }

    /// Returns the left-hand side operand of this `BinOp`.
    pub fn lhs(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(0)
    }

    /// Returns the right-hand side operand of this `BinOp`.
    pub fn rhs(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(1)
    }
}
// ANCHOR_END: binop_methods

/// Region terminator for structured control-flow ops (`if` / `while`).
// ANCHOR: yield_op_decl
#[pliron_op(
    name = "kaleidoscope.yield",
    format = "",
    interfaces = [IsTerminatorInterface, NResultsInterface<0>, NOpdsInterface<0>],
    verifier = "succ",
)]
pub struct YieldOp;
// ANCHOR_END: yield_op_decl

impl YieldOp {
    pub fn new(ctx: &mut Context) -> Self {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        YieldOp { op }
    }
}

/// Statement-form conditional with explicit `then` and `else` regions.
// ANCHOR: if_op_decl
#[pliron_op(
    name = "kaleidoscope.if",
    format = "$0 ` then ` region($0) ` else ` region($1)",
    interfaces = [
        NOpdsInterface<1>,
        NResultsInterface<0>,
        NRegionsInterface<2>,
        SingleBlockRegionInterface
    ],
    verifier = "succ",
)]
pub struct IfOp;
// ANCHOR_END: if_op_decl

impl IfOp {
    pub fn new(ctx: &mut Context, cond: Value) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![],
            vec![cond],
            vec![],
            2,
        );
        IfOp { op }
    }

    pub fn condition(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(0)
    }

    pub fn then_region(&self, ctx: &Context) -> Ptr<Region> {
        self.get_operation().deref(ctx).get_region(0)
    }

    pub fn else_region(&self, ctx: &Context) -> Ptr<Region> {
        self.get_operation().deref(ctx).get_region(1)
    }
}

/// Statement-form loop with one body region.
// ANCHOR: while_op_decl
#[pliron_op(
    name = "kaleidoscope.while",
    format = "`*`$0 ` do ` region($0)",
    interfaces = [
        OperandNOfType<0, PointerType>,
        NResultsInterface<0>,
        NRegionsInterface<1>,
        SingleBlockRegionInterface
    ],
    verifier = "succ",
)]
pub struct WhileOp;
// ANCHOR_END: while_op_decl

impl WhileOp {
    pub fn new(ctx: &mut Context, cond_ptr: Value) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![],
            vec![cond_ptr],
            vec![],
            1,
        );
        WhileOp { op }
    }

    pub fn cond_ptr(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(0)
    }

    pub fn body_region(&self, ctx: &Context) -> Ptr<Region> {
        self.get_operation().deref(ctx).get_region(0)
    }
}

/// Terminates a function-like region with a final value.
// ANCHOR: return_op_decl
#[pliron_op(
    name = "kaleidoscope.return",
    format = "$0",
    interfaces = [IsTerminatorInterface],
)]
pub struct ReturnOp;
// ANCHOR_END: return_op_decl

impl ReturnOp {
    pub fn new(ctx: &mut Context, value: Value) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![],
            vec![value],
            vec![],
            0,
        );
        ReturnOp { op }
    }

    pub fn value(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(0)
    }
}

// ANCHOR: return_op_manual_verifier
impl Verify for ReturnOp {
    fn verify(&self, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let n_operands = self.get_operation().deref(ctx).get_num_operands();
        if n_operands != 1 {
            return verify_err!(
                self.loc(ctx),
                "kaleidoscope.return expects exactly 1 operand, found {}",
                n_operands
            );
        }
        Ok(())
    }
}
// ANCHOR_END: return_op_manual_verifier

// ANCHOR: call_op_decl
#[pliron_op(
    name = "kaleidoscope.call",
    format = "`@`attr($callee, $IdentifierAttr) `(` operands(CharSpace(`,`)) `)` : type($0)",
    interfaces = [NResultsInterface<1>],
    attributes = (callee: IdentifierAttr),
    verifier = "succ",
)]
pub struct CallOp;
// ANCHOR_END: call_op_decl

impl CallOp {
    pub fn new(
        ctx: &mut Context,
        callee: IdentifierAttr,
        args: Vec<Value>,
        result_ty: Ptr<TypeObj>,
    ) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![result_ty],
            args,
            vec![],
            0,
        );
        let op = CallOp { op };
        op.set_attr_callee(ctx, callee);
        op
    }
}

// ANCHOR: fib_build_pseudocode
// Kaleidoscope pseudocode for the generated function:
//
// def main() {
//   var a = 0;
//   var b = 1;
//   var i = 0;
//   var n = 10;
//
//   while i < n {
//     var tmp = a + b;
//     a = b;
//     b = tmp;
//     i = i + 1;
//   }
//
//   return b;
// }
// ANCHOR_END: fib_build_pseudocode

#[cfg(test)]
mod tests {
    use super::*;

    // ANCHOR: fib_build_function
    #[test]
    fn build_fib_example() {
        use pliron::{
            basic_block::BasicBlock,
            builtin::{
                op_interfaces::OneResultInterface,
                ops::{FuncOp, ModuleOp},
                types::FunctionType,
            },
            debug_info::set_operation_result_name,
            irbuild::{
                inserter::{IRInserter, Inserter},
                listener::DummyListener,
            },
            op::verify_op,
            printable::Printable,
        };

        type OpInserter = IRInserter<DummyListener>;

        // ANCHOR: fib_build_setup
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);

        let module = ModuleOp::new(ctx, "fib_test".try_into().expect("valid module name"));
        let main_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let main_fn = FuncOp::new(
            ctx,
            "main".try_into().expect("valid function name"),
            main_ty,
        );
        module.append_operation(ctx, main_fn.get_operation(), 0);

        let entry = main_fn.get_entry_block(ctx);
        let mut ins = OpInserter::new_at_block_end(entry);
        // ANCHOR_END: fib_build_setup

        // ANCHOR: fib_build_slots
        // Mutable slots: a, b, tmp, i, and the iteration limit n.
        let slot_a = DeclOp::new(ctx, i64_ty.into());
        set_operation_result_name(ctx, slot_a.get_operation(), 0, "a".try_into().ok());
        ins.append_op(ctx, &slot_a);
        let slot_b = DeclOp::new(ctx, i64_ty.into());
        set_operation_result_name(ctx, slot_b.get_operation(), 0, "b".try_into().ok());
        ins.append_op(ctx, &slot_b);
        let slot_tmp = DeclOp::new(ctx, i64_ty.into());
        set_operation_result_name(ctx, slot_tmp.get_operation(), 0, "tmp".try_into().ok());
        ins.append_op(ctx, &slot_tmp);
        let slot_i = DeclOp::new(ctx, i64_ty.into());
        set_operation_result_name(ctx, slot_i.get_operation(), 0, "i".try_into().ok());
        ins.append_op(ctx, &slot_i);
        let slot_n = DeclOp::new(ctx, i64_ty.into());
        set_operation_result_name(ctx, slot_n.get_operation(), 0, "n".try_into().ok());
        ins.append_op(ctx, &slot_n);
        // A mutable slot for the loop condition (while cond_ptr do ...).
        let slot_cond_ptr = DeclOp::new(ctx, i64_ty.into());
        set_operation_result_name(
            ctx,
            slot_cond_ptr.get_operation(),
            0,
            "cond_ptr".try_into().ok(),
        );
        ins.append_op(ctx, &slot_cond_ptr);
        // ANCHOR_END: fib_build_slots

        // ANCHOR: fib_build_init_and_cond
        // Initialize: a=0, b=1, i=0, n=10.
        let c0 = ConstantOp::new_i64(ctx, 0);
        ins.append_op(ctx, &c0);
        let c1 = ConstantOp::new_i64(ctx, 1);
        ins.append_op(ctx, &c1);
        let c10 = ConstantOp::new_i64(ctx, 10);
        ins.append_op(ctx, &c10);

        let store_a0 = StoreOp::new(ctx, slot_a.get_result(ctx), c0.get_result(ctx));
        ins.append_op(ctx, &store_a0);
        let store_b1 = StoreOp::new(ctx, slot_b.get_result(ctx), c1.get_result(ctx));
        ins.append_op(ctx, &store_b1);
        let store_i0 = StoreOp::new(ctx, slot_i.get_result(ctx), c0.get_result(ctx));
        ins.append_op(ctx, &store_i0);
        let store_n10 = StoreOp::new(ctx, slot_n.get_result(ctx), c10.get_result(ctx));
        ins.append_op(ctx, &store_n10);

        // Loop condition i < n.
        let i_before = LoadOp::new(ctx, slot_i.get_result(ctx), i64_ty.into());
        ins.append_op(ctx, &i_before);
        let n_before = LoadOp::new(ctx, slot_n.get_result(ctx), i64_ty.into());
        ins.append_op(ctx, &n_before);
        let cond = BinOp::new(
            ctx,
            BinOpKind::Lt,
            i_before.get_result(ctx),
            n_before.get_result(ctx),
        );
        ins.append_op(ctx, &cond);
        let store_cond_ptr = StoreOp::new(ctx, slot_cond_ptr.get_result(ctx), cond.get_result(ctx));
        ins.append_op(ctx, &store_cond_ptr);

        let while_op = WhileOp::new(ctx, slot_cond_ptr.get_result(ctx));
        ins.append_op(ctx, &while_op);
        // ANCHOR_END: fib_build_init_and_cond

        // ANCHOR: fib_build_while_body
        let while_region = while_op.body_region(ctx);
        let while_block = BasicBlock::new(
            ctx,
            Some("while_body".try_into().expect("valid block name")),
            vec![],
        );
        while_block.insert_at_front(while_region, ctx);
        let mut while_ins = OpInserter::new_at_block_end(while_block);

        // tmp = a + b; a = b; b = tmp; i = i + 1
        let a_val = LoadOp::new(ctx, slot_a.get_result(ctx), i64_ty.into());
        while_ins.append_op(ctx, &a_val);
        let b_val = LoadOp::new(ctx, slot_b.get_result(ctx), i64_ty.into());
        while_ins.append_op(ctx, &b_val);
        let sum = BinOp::new(
            ctx,
            BinOpKind::Add,
            a_val.get_result(ctx),
            b_val.get_result(ctx),
        );
        while_ins.append_op(ctx, &sum);
        let store_tmp = StoreOp::new(ctx, slot_tmp.get_result(ctx), sum.get_result(ctx));
        while_ins.append_op(ctx, &store_tmp);

        let b_for_a = LoadOp::new(ctx, slot_b.get_result(ctx), i64_ty.into());
        while_ins.append_op(ctx, &b_for_a);
        let store_a = StoreOp::new(ctx, slot_a.get_result(ctx), b_for_a.get_result(ctx));
        while_ins.append_op(ctx, &store_a);

        let tmp_for_b = LoadOp::new(ctx, slot_tmp.get_result(ctx), i64_ty.into());
        while_ins.append_op(ctx, &tmp_for_b);
        let store_b = StoreOp::new(ctx, slot_b.get_result(ctx), tmp_for_b.get_result(ctx));
        while_ins.append_op(ctx, &store_b);

        let i_val = LoadOp::new(ctx, slot_i.get_result(ctx), i64_ty.into());
        while_ins.append_op(ctx, &i_val);
        let one_step = ConstantOp::new_i64(ctx, 1);
        while_ins.append_op(ctx, &one_step);
        let i_next = BinOp::new(
            ctx,
            BinOpKind::Add,
            i_val.get_result(ctx),
            one_step.get_result(ctx),
        );
        while_ins.append_op(ctx, &i_next);
        let store_i = StoreOp::new(ctx, slot_i.get_result(ctx), i_next.get_result(ctx));
        while_ins.append_op(ctx, &store_i);

        // i is updated, now do the comparison again
        let i_inc = LoadOp::new(ctx, slot_i.get_result(ctx), i64_ty.into());
        while_ins.append_op(ctx, &i_inc);
        let n_for_cond = LoadOp::new(ctx, slot_n.get_result(ctx), i64_ty.into());
        while_ins.append_op(ctx, &n_for_cond);
        let cond_next = BinOp::new(
            ctx,
            BinOpKind::Lt,
            i_inc.get_result(ctx),
            n_for_cond.get_result(ctx),
        );
        while_ins.append_op(ctx, &cond_next);
        let store_cond = StoreOp::new(
            ctx,
            slot_cond_ptr.get_result(ctx),
            cond_next.get_result(ctx),
        );
        while_ins.append_op(ctx, &store_cond);

        let while_yield = YieldOp::new(ctx);
        while_ins.append_op(ctx, &while_yield);
        // ANCHOR_END: fib_build_while_body

        // ANCHOR: fib_build_return
        let fib_result = LoadOp::new(ctx, slot_b.get_result(ctx), i64_ty.into());
        ins.append_op(ctx, &fib_result);
        let ret = ReturnOp::new(ctx, fib_result.get_result(ctx));
        ins.append_op(ctx, &ret);

        verify_op(&module, ctx).expect("constructed fibonacci IR should verify");
        println!("{}", module.get_operation().disp(ctx));
        // ANCHOR_END: fib_build_return
    }
    // ANCHOR_END: fib_build_function
}
