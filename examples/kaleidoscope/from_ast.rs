// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Translate from AST to IR using the dialect we defined in `dialect.rs`.
//!
//! The entry point is [`lower_function`], which takes a single [`Function`] AST
//! node and produces a [`FuncOp`] containing Kaleidoscope dialect ops.
//!
//! # Design
//! Every Kaleidoscope variable (including function parameters) is backed by a
//! [`DeclOp`] memory slot.  Reads become [`LoadOp`]s; writes become
//! [`StoreOp`]s.  Control flow uses [`IfOp`] (two regions) and [`WhileOp`]
//! (one region + a condition-pointer slot).  All values are 64-bit signless
//! integers (`i64`).

use pliron::{
    basic_block::BasicBlock,
    builtin::{
        attributes::IdentifierAttr,
        op_interfaces::OneResultInterface,
        ops::FuncOp,
        types::{FunctionType, IntegerType, Signedness},
    },
    context::Context,
    input_error,
    irbuild::{
        inserter::{IRInserter, Inserter},
        listener::DummyListener,
    },
    location::Location,
    op::Op,
    result::Result,
    std_deps::hash::FxHashMap,
    value::Value,
};

use crate::{
    ast::{BinOp as AstBinOp, Expr, Function, Stmt},
    dialect::{
        BinOp, BinOpKind, CallOp, ConstantOp, DeclOp, IfOp, LoadOp, ReturnOp, StoreOp, WhileOp,
        YieldOp,
    },
};

// ANCHOR: type_aliases
/// Inserter type used throughout this module.
type OpInserter = IRInserter<DummyListener>;

/// Maps variable names to the slot-pointer [`Value`] produced by their [`DeclOp`].
type VarMap = FxHashMap<String, Value>;
// ANCHOR_END: type_aliases

// ─── Public API ─────────────────────────────────────────────────────────────

/// Lower a single Kaleidoscope [`Function`] AST node into a [`FuncOp`].
///
/// All parameters and local variables are spilled into [`DeclOp`] memory slots.
/// The produced [`FuncOp`] has the signature `(i64, …) -> i64`.
// ANCHOR: lower_function
pub fn lower_function(ctx: &mut Context, func: &Function) -> Result<FuncOp> {
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);

    // Build the function type: all params are i64, single i64 return.
    let param_tys: Vec<_> = func.params.iter().map(|_| i64_ty.into()).collect();
    let func_ty = FunctionType::get(ctx, param_tys, vec![i64_ty.into()]);

    let func_op = FuncOp::new(
        ctx,
        func.name
            .as_str()
            .try_into()
            .expect("invalid function name"),
        func_ty,
    );

    let entry = func_op.get_entry_block(ctx);
    let mut ins = OpInserter::new_at_block_end(entry);
    let mut var_map = VarMap::default();

    // ANCHOR: lower_function_params
    // Spill each function parameter (block argument) into a mutable DeclOp slot.
    for (idx, param_name) in func.params.iter().enumerate() {
        let param_val = entry.deref(ctx).get_argument(idx);
        let slot = DeclOp::new(ctx, i64_ty.into());
        let slot_val = slot.get_result(ctx);
        ins.append_op(ctx, &slot);
        let store = StoreOp::new(ctx, slot_val, param_val);
        ins.append_op(ctx, &store);
        var_map.insert(param_name.clone(), slot_val);
    }
    // ANCHOR_END: lower_function_params

    lower_stmts(ctx, &mut ins, &mut var_map, &func.body)?;

    // ANCHOR: lower_function_fallback
    // If no terminator was emitted (e.g., function ends with an `if` where both
    // branches return), add a fallback return of 0 to satisfy the verifier.
    if entry.deref(ctx).get_terminator(ctx).is_none() {
        let zero = ConstantOp::new_i64(ctx, 0);
        let zero_val = zero.get_result(ctx);
        ins.append_op(ctx, &zero);
        let ret = ReturnOp::new(ctx, zero_val);
        ins.append_op(ctx, &ret);
    }
    // ANCHOR_END: lower_function_fallback
    Ok(func_op)
}
// ANCHOR_END: lower_function

// ─── Statement lowering ─────────────────────────────────────────────────────

/// Lower a list of statements. Returns `true` if the last emitted op is a
/// block terminator (i.e., a `return` was the last statement).
// ANCHOR: lower_stmts
fn lower_stmts(
    ctx: &mut Context,
    ins: &mut OpInserter,
    var_map: &mut VarMap,
    stmts: &[Stmt],
) -> Result<bool> {
    let mut terminated = false;
    for stmt in stmts {
        terminated = lower_stmt(ctx, ins, var_map, stmt)?;
    }
    Ok(terminated)
}
// ANCHOR_END: lower_stmts

/// Lower one statement. Returns `true` if it emitted a block terminator.
fn lower_stmt(
    ctx: &mut Context,
    ins: &mut OpInserter,
    var_map: &mut VarMap,
    stmt: &Stmt,
) -> Result<bool> {
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);

    match stmt {
        // ── var name; / var name = expr; ──────────────────────────────────
        // ANCHOR: lower_stmt_vardecl
        Stmt::VarDecl { name, init } => {
            let slot = DeclOp::new(ctx, i64_ty.into());
            let slot_val = slot.get_result(ctx);
            ins.append_op(ctx, &slot);
            var_map.insert(name.clone(), slot_val);

            if let Some(init_expr) = init {
                let val = lower_expr(ctx, ins, var_map, init_expr)?;
                let store = StoreOp::new(ctx, slot_val, val);
                ins.append_op(ctx, &store);
            }
            Ok(false)
        }
        // ANCHOR_END: lower_stmt_vardecl

        // ── name = expr; ──────────────────────────────────────────────────
        // ANCHOR: lower_stmt_assign
        Stmt::Assign { name, value } => {
            let val = lower_expr(ctx, ins, var_map, value)?;
            let slot = *var_map.get(name.as_str()).ok_or_else(|| {
                input_error!(
                    Location::Unknown,
                    "assignment to undeclared variable: {name}"
                )
            })?;
            let store = StoreOp::new(ctx, slot, val);
            ins.append_op(ctx, &store);
            Ok(false)
        }
        // ANCHOR_END: lower_stmt_assign

        // ── return expr; ──────────────────────────────────────────────────
        // ANCHOR: lower_stmt_return
        Stmt::Return(expr) => {
            let val = lower_expr(ctx, ins, var_map, expr)?;
            let ret = ReturnOp::new(ctx, val);
            ins.append_op(ctx, &ret);
            Ok(true) // ReturnOp is a block terminator
        }
        // ANCHOR_END: lower_stmt_return

        // ── if cond { then } else { else } ───────────────────────────────
        // ANCHOR: lower_stmt_if
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            let cond_val = lower_expr(ctx, ins, var_map, cond)?;
            let if_op = IfOp::new(ctx, cond_val);
            ins.append_op(ctx, &if_op);

            // Then region: add YieldOp only if the branch didn't terminate.
            let then_block = BasicBlock::new(ctx, None, vec![]);
            then_block.insert_at_front(if_op.then_region(ctx), ctx);
            let mut then_ins = OpInserter::new_at_block_end(then_block);
            let mut then_vars = var_map.clone();
            let then_terminated = lower_stmts(ctx, &mut then_ins, &mut then_vars, then_body)?;
            if !then_terminated {
                let then_yield = YieldOp::new(ctx);
                then_ins.append_op(ctx, &then_yield);
            }

            // Else region: add YieldOp only if the branch didn't terminate.
            let else_block = BasicBlock::new(ctx, None, vec![]);
            else_block.insert_at_front(if_op.else_region(ctx), ctx);
            let mut else_ins = OpInserter::new_at_block_end(else_block);
            let mut else_vars = var_map.clone();
            let else_terminated = lower_stmts(ctx, &mut else_ins, &mut else_vars, else_body)?;
            if !else_terminated {
                let else_yield = YieldOp::new(ctx);
                else_ins.append_op(ctx, &else_yield);
            }

            Ok(false) // IfOp itself is not a terminator in the outer block
        }
        // ANCHOR_END: lower_stmt_if

        // ── while cond { body } ───────────────────────────────────────────
        //
        // WhileOp takes a *pointer* (DeclOp slot) whose i64 value is checked
        // before each iteration.  We compute the condition before the loop and
        // at the end of every iteration, storing the result into the slot.
        // ANCHOR: lower_stmt_while
        Stmt::While { cond, body } => {
            // Allocate the condition slot in the outer block.
            let cond_slot = DeclOp::new(ctx, i64_ty.into());
            let cond_slot_val = cond_slot.get_result(ctx);
            ins.append_op(ctx, &cond_slot);

            // Compute the initial condition and store it.
            let init_cond = lower_expr(ctx, ins, var_map, cond)?;
            let init_store = StoreOp::new(ctx, cond_slot_val, init_cond);
            ins.append_op(ctx, &init_store);

            let while_op = WhileOp::new(ctx, cond_slot_val);
            ins.append_op(ctx, &while_op);

            // Build the loop body.
            let body_block = BasicBlock::new(ctx, None, vec![]);
            body_block.insert_at_front(while_op.body_region(ctx), ctx);
            let mut body_ins = OpInserter::new_at_block_end(body_block);
            let mut body_vars = var_map.clone();
            let body_terminated = lower_stmts(ctx, &mut body_ins, &mut body_vars, body)?;

            if !body_terminated {
                // Re-evaluate the condition at the end of the body and update the slot.
                let next_cond = lower_expr(ctx, &mut body_ins, &body_vars, cond)?;
                let next_store = StoreOp::new(ctx, cond_slot_val, next_cond);
                body_ins.append_op(ctx, &next_store);
                let body_yield = YieldOp::new(ctx);
                body_ins.append_op(ctx, &body_yield);
            }

            Ok(false) // WhileOp itself is not a terminator in the outer block
        }
        // ANCHOR_END: lower_stmt_while

        // ── expr; (side-effect expression statement) ──────────────────────
        // ANCHOR: lower_stmt_expr
        Stmt::Expr(expr) => {
            lower_expr(ctx, ins, var_map, expr)?;
            Ok(false)
        } // ANCHOR_END: lower_stmt_expr
    }
}

// ─── Expression lowering ────────────────────────────────────────────────────

/// Lower an expression, inserting ops via `ins`, and return the resulting [`Value`].
// ANCHOR: lower_expr
fn lower_expr(
    ctx: &mut Context,
    ins: &mut OpInserter,
    var_map: &VarMap,
    expr: &Expr,
) -> Result<Value> {
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signless);

    match expr {
        // ── integer literal ───────────────────────────────────────────────
        // ANCHOR: lower_expr_integer
        Expr::Integer(n) => {
            let op = ConstantOp::new_i64(ctx, *n);
            let val = op.get_result(ctx);
            ins.append_op(ctx, &op);
            Ok(val)
        }
        // ANCHOR_END: lower_expr_integer

        // ── variable reference ────────────────────────────────────────────
        // ANCHOR: lower_expr_variable
        Expr::Variable(name) => {
            let slot = *var_map.get(name.as_str()).ok_or_else(|| {
                input_error!(
                    Location::Unknown,
                    "reference to undeclared variable: {name}"
                )
            })?;
            let load = LoadOp::new(ctx, slot, i64_ty.into());
            let val = load.get_result(ctx);
            ins.append_op(ctx, &load);
            Ok(val)
        }
        // ANCHOR_END: lower_expr_variable

        // ── binary operation ──────────────────────────────────────────────
        // ANCHOR: lower_expr_binop
        Expr::BinOp { op, lhs, rhs } => {
            let lhs_val = lower_expr(ctx, ins, var_map, lhs)?;
            let rhs_val = lower_expr(ctx, ins, var_map, rhs)?;
            let kind = ast_binop_to_kind(op);
            let bin_op = BinOp::new(ctx, kind, lhs_val, rhs_val);
            let val = bin_op.get_result(ctx);
            ins.append_op(ctx, &bin_op);
            Ok(val)
        }
        // ANCHOR_END: lower_expr_binop

        // ── function call ─────────────────────────────────────────────────
        // ANCHOR: lower_expr_call
        Expr::Call { callee, args } => {
            let mut arg_vals = Vec::with_capacity(args.len());
            for a in args {
                arg_vals.push(lower_expr(ctx, ins, var_map, a)?);
            }
            let callee_id = callee.as_str().try_into().expect("valid callee identifier");
            let call_op = CallOp::new(ctx, IdentifierAttr::new(callee_id), arg_vals, i64_ty.into());
            let val = call_op.get_operation().deref(ctx).get_result(0);
            let val = { val }; // reborrow to release ctx ref before append
            ins.append_op(ctx, &call_op);
            Ok(val)
        } // ANCHOR_END: lower_expr_call
    }
}
// ANCHOR_END: lower_expr

// ─── Helpers ────────────────────────────────────────────────────────────────

fn ast_binop_to_kind(op: &AstBinOp) -> BinOpKind {
    match op {
        AstBinOp::Add => BinOpKind::Add,
        AstBinOp::Sub => BinOpKind::Sub,
        AstBinOp::Mul => BinOpKind::Mul,
        AstBinOp::Lt => BinOpKind::Lt,
        AstBinOp::Gt => BinOpKind::Gt,
        AstBinOp::Le => BinOpKind::Le,
        AstBinOp::Ge => BinOpKind::Ge,
        AstBinOp::Eq => BinOpKind::Eq,
        AstBinOp::Ne => BinOpKind::Ne,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use pliron::{
        builtin::{op_interfaces::SingleBlockRegionInterface, ops::ModuleOp},
        context::Context,
        op::{Op, verify_op},
        printable::Printable,
    };

    use crate::ast::parse_program;

    use super::lower_function;

    // ANCHOR: lower_test_helper
    fn lower_program(src: &str) -> String {
        let funcs = parse_program(src).expect("parse error");
        let ctx = &mut Context::new();
        let module = ModuleOp::new(ctx, "test".try_into().expect("valid module name"));
        for func in &funcs {
            let func_op = lower_function(ctx, func).expect("lowering failed");
            module.append_operation(ctx, func_op.get_operation(), 0);
        }
        verify_op(&module, ctx).expect("IR verification failed");
        format!("{}", module.get_operation().disp(ctx))
    }
    // ANCHOR_END: lower_test_helper

    #[test]
    fn fibonacci_from_ast() {
        let src = std::fs::read_to_string("examples/kaleidoscope/fibonacci.kal")
            .expect("failed to read fibonacci.kal");
        let ir = lower_program(&src);
        println!("{ir}");
        // Smoke-check: both functions appear in the output.
        assert!(ir.contains("@main"));
        assert!(ir.contains("@fib"));
    }

    #[test]
    fn factorial_from_ast() {
        let src = std::fs::read_to_string("examples/kaleidoscope/factorial.kal")
            .expect("failed to read factorial.kal");
        let ir = lower_program(&src);
        println!("{ir}");
        assert!(ir.contains("@main"));
        assert!(ir.contains("@factorial"));
    }

    #[test]
    fn inline_fibonacci_from_ast() {
        // Iterative fibonacci matching the hand-written build_fib_example test.
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
        let ir = lower_program(src);
        println!("{ir}");
        assert!(ir.contains("@main"));
    }
}
