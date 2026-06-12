//! Translate from Kaleidoscope dialect IR to LLVM dialect IR.
//!
//! The entry point is [`lower_module`], which applies the
//! [`DialectConversion`] infrastructure to convert a [`ModuleOp`] containing
//! Kaleidoscope dialect ops into one containing only LLVM dialect ops.
//!
//! # Design
//! Each Kaleidoscope op implements the [`ToLLVMDialect`] op-interface.
//! A [`KalToLLVM`] conversion driver matches all ops that implement this
//! interface and delegates each rewrite to the op itself.
//!
//! * `ConstantOp` -> `llvm.constant`
//! * `DeclOp`     -> `llvm.alloca`
//! * `LoadOp`     -> `llvm.load`
//! * `StoreOp`    -> `llvm.store`
//! * `BinOp`      -> `llvm.add`/`sub`/`mul` or `llvm.icmp` + `llvm.sext`
//! * `CallOp`     -> `llvm.call`
//! * `ReturnOp`   -> `llvm.return`
//! * `YieldOp`    -> erased (handled by parent IfOp / WhileOp)
//! * `IfOp`       -> CFG: then / else / merge blocks
//! * `WhileOp`    -> CFG: header / body / exit blocks

use awint::bw;

use pliron::{
    builtin::{
        self,
        attributes::IntegerAttr,
        op_interfaces::{
            CallOpCallable, OneRegionInterface, OneResultInterface, SymbolOpInterface,
        },
        ops::{ConstantOp as BuiltinConstantOp, ModuleOp},
        types::{IntegerType, Signedness},
    },
    context::{Context, Ptr},
    derive::op_interface_impl,
    irbuild::{
        IRStatus,
        dialect_conversion::{
            DialectConversion, DialectConversionRewriter, OperandsInfo, apply_dialect_conversion,
        },
        inserter::{BlockInsertionPoint, Inserter, OpInsertionPoint},
        rewriter::Rewriter,
    },
    linked_list::ContainsLinkedList,
    op::{Op, op_cast, op_impls},
    operation::Operation,
    result::Result,
    r#type::TypeObj,
    utils::apint::APInt,
    value::Value,
};
use pliron_llvm::{
    ToLLVMDialect,
    attributes::{ICmpPredicateAttr, IntegerOverflowFlagsAttr},
    op_interfaces::{CastOpInterface, IntBinArithOpWithOverflowFlag},
    ops::{
        AddOp, AllocaOp, BrOp, CallOp as LlvmCallOp, CondBrOp, ICmpOp, LoadOp as LlvmLoadOp, MulOp,
        ReturnOp as LlvmReturnOp, SExtOp, StoreOp as LlvmStoreOp, SubOp,
    },
    types::FuncType,
};

use crate::dialect::{
    BinOp, BinOpKind, CallOp as KalCallOp, ConstantOp as KalConstantOp, DeclOp as KalDeclOp,
    IfOp as KalIfOp, LoadOp as KalLoadOp, ReturnOp as KalReturnOp, StoreOp as KalStoreOp,
    WhileOp as KalWhileOp, YieldOp as KalYieldOp,
};

// ─── DialectConversion driver ───────────────────────────────────────────────

/// Conversion driver: matches any Kaleidoscope op that implements
/// [`ToLLVMDialect`] and delegates the rewrite to the op itself.
// ANCHOR: kal_to_llvm_driver
pub struct KalToLLVM;

impl DialectConversion for KalToLLVM {
    fn can_convert_op(&self, ctx: &Context, op: Ptr<Operation>) -> bool {
        op_impls::<dyn ToLLVMDialect>(&*Operation::get_op_dyn(op, ctx))
            || Operation::get_op::<builtin::ops::FuncOp>(op, ctx).is_some()
    }

    fn rewrite(
        &mut self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        op: Ptr<Operation>,
        operands_info: &OperandsInfo,
    ) -> Result<()> {
        if let Some(func_op) = Operation::get_op::<builtin::ops::FuncOp>(op, ctx) {
            // Convert from builtin.func to llvm.func by updating the function type and argument types.
            return lower_func_op_to_llvm(&func_op, ctx, rewriter);
        }
        let op_dyn = Operation::get_op_dyn(op, ctx);
        let to_llvm_op = op_cast::<dyn ToLLVMDialect>(&*op_dyn)
            .expect("Matched Op must implement ToLLVMDialect");
        to_llvm_op.rewrite(ctx, rewriter, operands_info)
    }
}
// ANCHOR_END: kal_to_llvm_driver

// ─── Public API ─────────────────────────────────────────────────────────────

// ANCHOR: lower_module
/// Lower a [`ModuleOp`] containing Kaleidoscope dialect ops in place.
///
/// Uses the [`DialectConversion`] infrastructure: each Kaleidoscope op
/// implements [`ToLLVMDialect`] and knows how to lower itself to LLVM ops.
pub fn lower_module(ctx: &mut Context, module: ModuleOp) -> Result<IRStatus> {
    apply_dialect_conversion(ctx, &mut KalToLLVM, module.get_operation())
}
// ANCHOR_END: lower_module

// ─── ToLLVMDialect implementations ─────────────────────────────────────────

// ── kaleidoscope.constant -> llvm.constant ───────────────────────────────────
// ANCHOR: constant_to_llvm
#[op_interface_impl]
impl ToLLVMDialect for KalConstantOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let val_attr = self.value_attr(ctx);
        let llvm_const = BuiltinConstantOp::new(ctx, Box::new(val_attr));
        let new_result = llvm_const.get_result(ctx);
        rewriter.insert_op(ctx, &llvm_const);
        rewriter.replace_operation_with_values(ctx, self.get_operation(), vec![new_result]);
        Ok(())
    }
}
// ANCHOR_END: constant_to_llvm

// ── kaleidoscope.decl -> llvm.alloca ─────────────────────────────────────────
// ANCHOR: decl_to_llvm
#[op_interface_impl]
impl ToLLVMDialect for KalDeclOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let i32_ty = IntegerType::get(ctx, 32, Signedness::Signless);
        let elem_ty = self.variable_type(ctx);
        let size_attr = IntegerAttr::new(i32_ty, APInt::from_i32(1, bw(32)));
        let size_const = BuiltinConstantOp::new(ctx, Box::new(size_attr));
        let size_val = size_const.get_result(ctx);
        rewriter.insert_op(ctx, &size_const);
        let alloca = AllocaOp::new(ctx, elem_ty, size_val);
        let alloca_ptr = alloca.get_result(ctx);
        rewriter.insert_op(ctx, &alloca);
        rewriter.replace_operation_with_values(ctx, self.get_operation(), vec![alloca_ptr]);
        Ok(())
    }
}
// ANCHOR_END: decl_to_llvm

// ── kaleidoscope.load -> llvm.load ───────────────────────────────────────────
// ANCHOR: load_to_llvm
#[op_interface_impl]
impl ToLLVMDialect for KalLoadOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        operands_info: &OperandsInfo,
    ) -> Result<()> {
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);
        // `operands_info` carries the already-converted slot operand.
        let slot = operands_info
            .lookup_most_recent_type(self.slot(ctx))
            .map_or(self.slot(ctx), |_| self.slot(ctx));
        // Operand was already updated in-place by the framework.
        let load = LlvmLoadOp::new(ctx, slot, i64_ty.into());
        let result = load.get_result(ctx);
        rewriter.insert_op(ctx, &load);
        rewriter.replace_operation_with_values(ctx, self.get_operation(), vec![result]);
        Ok(())
    }
}
// ANCHOR_END: load_to_llvm

// ── kaleidoscope.store -> llvm.store ─────────────────────────────────────────
// ANCHOR: store_to_llvm
#[op_interface_impl]
impl ToLLVMDialect for KalStoreOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let slot = self.slot(ctx);
        let value = self.stored_value(ctx);
        let store = LlvmStoreOp::new(ctx, value, slot);
        rewriter.insert_op(ctx, &store);
        rewriter.erase_operation(ctx, self.get_operation());
        Ok(())
    }
}
// ANCHOR_END: store_to_llvm

// ── kaleidoscope.binop -> llvm arithmetic / comparison ───────────────────────
// ANCHOR: binop_to_llvm
#[op_interface_impl]
impl ToLLVMDialect for BinOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);
        let lhs = self.lhs(ctx);
        let rhs = self.rhs(ctx);
        let kind = self.kind(ctx);

        let result = match kind {
            BinOpKind::Add => {
                let op = AddOp::new_with_overflow_flag(
                    ctx,
                    lhs,
                    rhs,
                    IntegerOverflowFlagsAttr::default(),
                );
                let r = op.get_result(ctx);
                rewriter.insert_op(ctx, &op);
                r
            }
            BinOpKind::Sub => {
                let op = SubOp::new_with_overflow_flag(
                    ctx,
                    lhs,
                    rhs,
                    IntegerOverflowFlagsAttr::default(),
                );
                let r = op.get_result(ctx);
                rewriter.insert_op(ctx, &op);
                r
            }
            BinOpKind::Mul => {
                let op = MulOp::new_with_overflow_flag(
                    ctx,
                    lhs,
                    rhs,
                    IntegerOverflowFlagsAttr::default(),
                );
                let r = op.get_result(ctx);
                rewriter.insert_op(ctx, &op);
                r
            }
            _ => {
                // Comparison: ICmpOp yields i1; sign-extend to i64.
                let pred = binop_kind_to_icmp_pred(kind);
                let icmp = ICmpOp::new(ctx, pred, lhs, rhs);
                let cmp_i1 = icmp.get_result(ctx);
                rewriter.insert_op(ctx, &icmp);
                let sext = SExtOp::new(ctx, cmp_i1, i64_ty.into());
                let r = sext.get_result(ctx);
                rewriter.insert_op(ctx, &sext);
                r
            }
        };
        rewriter.replace_operation_with_values(ctx, self.get_operation(), vec![result]);
        Ok(())
    }
}
// ANCHOR_END: binop_to_llvm

// ── kaleidoscope.call -> llvm.call ───────────────────────────────────────────
// ANCHOR: call_to_llvm
#[op_interface_impl]
impl ToLLVMDialect for KalCallOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);
        let callee_attr = self
            .get_attr_callee(ctx)
            .expect("CallOp must have callee attribute")
            .clone();
        let callee_ident = pliron::identifier::Identifier::from(callee_attr);
        let n_args = self.get_operation().deref(ctx).get_num_operands();
        let args: Vec<Value> = (0..n_args)
            .map(|i| self.get_operation().deref(ctx).get_operand(i))
            .collect();
        let arg_types: Vec<Ptr<TypeObj>> = (0..n_args).map(|_| i64_ty.into()).collect();
        let llvm_func_ty = FuncType::get(ctx, i64_ty.into(), arg_types, false);
        let llvm_call = LlvmCallOp::new(
            ctx,
            CallOpCallable::Direct(callee_ident),
            llvm_func_ty,
            args,
        );
        let result = llvm_call.get_result(ctx);
        rewriter.insert_op(ctx, &llvm_call);
        rewriter.replace_operation_with_values(ctx, self.get_operation(), vec![result]);
        Ok(())
    }
}
// ANCHOR_END: call_to_llvm

// ── kaleidoscope.return -> llvm.return ───────────────────────────────────────
// ANCHOR: return_to_llvm
#[op_interface_impl]
impl ToLLVMDialect for KalReturnOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let val = self.value(ctx);
        let ret = LlvmReturnOp::new(ctx, Some(val));
        rewriter.insert_op(ctx, &ret);
        rewriter.erase_operation(ctx, self.get_operation());
        Ok(())
    }
}
// ANCHOR_END: return_to_llvm

// ── kaleidoscope.yield -> erase ──────────────────────────────────────────────
// YieldOp is an IsTerminatorInterface impl, so it must be handled.
// It's handled by the parent IfOp/WhileOp's rewrite, but may also be
// matched here; in that case just erase it.
#[op_interface_impl]
impl ToLLVMDialect for KalYieldOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        rewriter.erase_operation(ctx, self.get_operation());
        Ok(())
    }
}

// ── kaleidoscope.if -> CFG: then / else / merge blocks ───────────────────────
// ANCHOR: if_to_llvm
#[op_interface_impl]
impl ToLLVMDialect for KalIfOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);
        let cond = self.condition(ctx);
        let then_region = self.then_region(ctx);
        let else_region = self.else_region(ctx);

        // The entry block of each region is the single block in the region.
        let then_entry = then_region
            .deref(ctx)
            .get_head()
            .expect("IfOp then_region must have a block");
        let else_entry = else_region
            .deref(ctx)
            .get_head()
            .expect("IfOp else_region must have a block");

        let then_term = then_entry
            .deref(ctx)
            .get_terminator(ctx)
            .expect("then block must have a terminator");
        let else_term = else_entry
            .deref(ctx)
            .get_terminator(ctx)
            .expect("else block must have a terminator");

        // Convert the i64 condition to i1 via `icmp ne cond, 0`.
        let zero_attr = IntegerAttr::new(i64_ty, APInt::from_i64(0, bw(64)));
        let zero_const = BuiltinConstantOp::new(ctx, Box::new(zero_attr));
        let zero_val = zero_const.get_result(ctx);
        rewriter.insert_op(ctx, &zero_const);
        let cmp = ICmpOp::new(ctx, ICmpPredicateAttr::NE, cond, zero_val);
        let cmp_i1 = cmp.get_result(ctx);
        rewriter.insert_op(ctx, &cmp);

        // Split the current block at the IfOp position to create the merge block.
        let pre_if_block = self
            .get_operation()
            .deref(ctx)
            .get_parent_block()
            .expect("IfOp must be in a block");
        let merge_block = rewriter.split_block(
            ctx,
            pre_if_block,
            OpInsertionPoint::BeforeOperation(self.get_operation()),
            Some("if_merge".try_into().unwrap()),
        );

        // Emit conditional branch in pre_if_block.
        rewriter.set_insertion_point(OpInsertionPoint::AtBlockEnd(pre_if_block));
        let cond_br = CondBrOp::new(ctx, cmp_i1, then_entry, vec![], else_entry, vec![]);
        rewriter.insert_op(ctx, &cond_br);

        // Replace YieldOp in then-branch with branch to merge.
        if Operation::is_op::<KalYieldOp>(then_term, ctx) {
            rewriter.set_insertion_point(OpInsertionPoint::BeforeOperation(then_term));
            let then_br = BrOp::new(ctx, merge_block, vec![]);
            rewriter.insert_op(ctx, &then_br);
            rewriter.erase_operation(ctx, then_term);
        }

        // Replace YieldOp in else-branch with branch to merge.
        if Operation::is_op::<KalYieldOp>(else_term, ctx) {
            rewriter.set_insertion_point(OpInsertionPoint::BeforeOperation(else_term));
            let else_br = BrOp::new(ctx, merge_block, vec![]);
            rewriter.insert_op(ctx, &else_br);
            rewriter.erase_operation(ctx, else_term);
        }

        // Inline both regions after the pre_if_block.
        rewriter.inline_region(
            ctx,
            then_region,
            BlockInsertionPoint::AfterBlock(pre_if_block),
        );
        rewriter.inline_region(
            ctx,
            else_region,
            BlockInsertionPoint::AfterBlock(then_entry),
        );

        // The IfOp itself has no results, so just erase it.
        rewriter.erase_operation(ctx, self.get_operation());
        Ok(())
    }
}
// ANCHOR_END: if_to_llvm

// ── kaleidoscope.while -> CFG: header / body / exit blocks ───────────────────
// ANCHOR: while_to_llvm
#[op_interface_impl]
impl ToLLVMDialect for KalWhileOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);
        let cond_ptr = self.cond_ptr(ctx);
        let body_region = self.body_region(ctx);
        let body_entry = body_region
            .deref(ctx)
            .get_head()
            .expect("WhileOp body_region must have a block");

        // Erase the YieldOp at the end of the body.
        let while_term = body_entry
            .deref(ctx)
            .get_terminator(ctx)
            .expect("body block must have a terminator");

        // The block containing the WhileOp is split to create the exit block.
        let pre_while_block = self
            .get_operation()
            .deref(ctx)
            .get_parent_block()
            .expect("WhileOp must be in a block");
        let exit_block = rewriter.split_block(
            ctx,
            pre_while_block,
            OpInsertionPoint::BeforeOperation(self.get_operation()),
            Some("while_exit".try_into().unwrap()),
        );

        // Create the header block.
        let header_block = rewriter.create_block(
            ctx,
            BlockInsertionPoint::AfterBlock(pre_while_block),
            Some("while_header".try_into().unwrap()),
            vec![],
        );

        // Emit an unconditional branch into the header from pre_while_block.
        rewriter.set_insertion_point(OpInsertionPoint::AtBlockEnd(pre_while_block));
        let br_to_header = BrOp::new(ctx, header_block, vec![]);
        rewriter.insert_op(ctx, &br_to_header);

        // Header: load condition, compare to zero, cond_br to body or exit.
        rewriter.set_insertion_point(OpInsertionPoint::AtBlockEnd(header_block));
        let cond_load = LlvmLoadOp::new(ctx, cond_ptr, i64_ty.into());
        let cond_i64 = cond_load.get_result(ctx);
        rewriter.insert_op(ctx, &cond_load);
        let zero_attr = IntegerAttr::new(i64_ty, APInt::from_i64(0, bw(64)));
        let zero_const = BuiltinConstantOp::new(ctx, Box::new(zero_attr));
        let zero_val = zero_const.get_result(ctx);
        rewriter.insert_op(ctx, &zero_const);
        let cmp = ICmpOp::new(ctx, ICmpPredicateAttr::NE, cond_i64, zero_val);
        let cmp_i1 = cmp.get_result(ctx);
        rewriter.insert_op(ctx, &cmp);
        let cond_br = CondBrOp::new(ctx, cmp_i1, body_entry, vec![], exit_block, vec![]);
        rewriter.insert_op(ctx, &cond_br);

        // Replace YieldOp at end of body with back-edge to header.
        if Operation::is_op::<KalYieldOp>(while_term, ctx) {
            rewriter.set_insertion_point(OpInsertionPoint::BeforeOperation(while_term));
            let back_edge = BrOp::new(ctx, header_block, vec![]);
            rewriter.insert_op(ctx, &back_edge);
            rewriter.erase_operation(ctx, while_term);
        }

        // Inline the body region after the header block.
        rewriter.inline_region(
            ctx,
            body_region,
            BlockInsertionPoint::AfterBlock(header_block),
        );

        // Erase the WhileOp.
        rewriter.erase_operation(ctx, self.get_operation());
        Ok(())
    }
}
// ANCHOR_END: while_to_llvm

// Convert from builtin.func to llvm.func by updating the function type and argument types.
fn lower_func_op_to_llvm(
    func_op: &builtin::ops::FuncOp,
    ctx: &mut Context,
    rewriter: &mut DialectConversionRewriter,
) -> Result<()> {
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);
    let func_name = func_op.get_symbol_name(ctx);
    let func_entry = func_op.get_entry_block(ctx);

    // All args are i64, and the return type is i64.
    let n_args = func_op.get_entry_block(ctx).deref(ctx).get_num_arguments();
    let arg_types: Vec<Ptr<TypeObj>> = (0..n_args).map(|_| i64_ty.into()).collect();
    let llvm_func_ty = FuncType::get(ctx, i64_ty.into(), arg_types, false);
    let llvm_func_op = pliron_llvm::ops::FuncOp::new(ctx, func_name, llvm_func_ty);
    let llvm_func_op_ptr = llvm_func_op.get_operation();
    let llvm_entry = llvm_func_op.get_or_create_entry_block(ctx);
    rewriter.insert_op(ctx, &llvm_func_op);

    // Move the region from the original func_op to the new llvm.func op.
    rewriter.inline_region(
        ctx,
        func_op.get_region(ctx),
        BlockInsertionPoint::AfterBlock(llvm_entry),
    );

    // Branch from the new entry block to the original entry block
    // (now inlined after the new entry block).
    let args: Vec<_> = llvm_entry.deref(ctx).arguments().collect();
    let br = BrOp::new(ctx, func_entry, args);
    rewriter.set_insertion_point(OpInsertionPoint::AtBlockEnd(llvm_entry));
    rewriter.insert_op(ctx, &br);

    // Replace the original FuncOp with the new llvm.func op.
    rewriter.replace_operation(ctx, func_op.get_operation(), llvm_func_op_ptr);
    Ok(())
}

// ─── Helper ────────────────────────────────────────────────────────────────

/// Map a comparison [`BinOpKind`] to the corresponding [`ICmpPredicateAttr`].
fn binop_kind_to_icmp_pred(kind: BinOpKind) -> ICmpPredicateAttr {
    match kind {
        BinOpKind::Lt => ICmpPredicateAttr::SLT,
        BinOpKind::Gt => ICmpPredicateAttr::SGT,
        BinOpKind::Le => ICmpPredicateAttr::SLE,
        BinOpKind::Ge => ICmpPredicateAttr::SGE,
        BinOpKind::Eq => ICmpPredicateAttr::EQ,
        BinOpKind::Ne => ICmpPredicateAttr::NE,
        _ => panic!("binop_kind_to_icmp_pred: not a comparison op"),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use pliron::{
        builtin::{op_interfaces::SingleBlockRegionInterface, ops::ModuleOp},
        context::Context,
        op::Op,
        operation::verify_operation,
        printable::Printable,
    };

    use crate::{ast::parse_program, from_ast::lower_function};

    use super::lower_module;

    // ANCHOR: lower_to_llvm_test_helper
    fn lower_to_llvm(src: &str) -> String {
        let funcs = parse_program(src).expect("parse error");
        let ctx = &mut Context::new();
        let module = ModuleOp::new(ctx, "test".try_into().expect("valid module name"));
        for func in &funcs {
            let func_op = lower_function(ctx, func).expect("kaleidoscope lowering failed");
            module.append_operation(ctx, func_op.get_operation(), 0);
        }
        let module_op_ptr = module.get_operation();
        lower_module(ctx, module).expect("LLVM lowering failed");
        verify_operation(module_op_ptr, ctx)
            .expect("module verification failed after LLVM lowering");
        format!("{}", module_op_ptr.disp(ctx))
    }
    // ANCHOR_END: lower_to_llvm_test_helper

    #[test]
    fn fibonacci_to_llvm() {
        let src = std::fs::read_to_string("examples/kaleidoscope/fibonacci.kal")
            .expect("failed to read fibonacci.kal");
        let ir = lower_to_llvm(&src);
        println!("{ir}");
        assert!(ir.contains("@main"));
        assert!(ir.contains("@fib"));
    }

    #[test]
    fn factorial_to_llvm() {
        let src = std::fs::read_to_string("examples/kaleidoscope/factorial.kal")
            .expect("failed to read factorial.kal");
        let ir = lower_to_llvm(&src);
        println!("{ir}");
        assert!(ir.contains("@main"));
        assert!(ir.contains("@factorial"));
    }

    #[test]
    fn inline_fibonacci_to_llvm() {
        let src = "
            def main() {
                var a = 0;
                var b = 1;
                var i = 0;
                var n = 10;
                while i < n {
                    var tmp = a + b;
                    a = b;
                    b = tmp;
                    i = i + 1;
                }
                return b;
            }
        ";
        let ir = lower_to_llvm(src);
        println!("{ir}");
        // The WhileOp is converted to a CFG loop with a header block.
        assert!(ir.contains("while_header"));
    }

    #[test]
    fn if_else_to_llvm() {
        let src = "
            def abs(x) {
                var result = 0;
                if x < 0 {
                    result = 0 - x;
                } else {
                    result = x;
                }
                return result;
            }
        ";
        let ir = lower_to_llvm(src);
        println!("{ir}");
        assert!(ir.contains("@abs"));
        // The IfOp is converted to CFG with conditional branching.
        assert!(ir.contains("llvm.cond_br"));
    }
}
