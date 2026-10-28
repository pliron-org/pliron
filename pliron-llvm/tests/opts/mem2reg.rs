// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Test that llvm operations implement the mem2reg interfaces
//! [PromotableAllocationInterface] and [PromotableOpInterface] correctly

use expect_test::expect;
use pliron::{
    context::Context, init_env_logger_for_tests, irbuild::IRStatus, op::Op,
    operation::verify_operation, opts::mem2reg::mem2reg, pass::AnalysisManager,
    printable::Printable, result::Result,
};

use pliron_llvm::ops::FuncOp;

use crate::common;

fn run_mem2reg_on_text(input: &str) -> Result<(IRStatus, String)> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();
    let op: FuncOp = common::parse_op_verify(ctx, input)?;

    let mut analyses = AnalysisManager::default();
    let status = mem2reg(op.get_operation(), ctx, &mut analyses)?;
    let after = op.disp(ctx).to_string();
    log::trace!("After mem2reg:\n{}", after);
    verify_operation(op.get_operation(), ctx)?;
    Ok((status, after))
}

/// An alloca whose pointer is stored into the alloca itself must stay in memory,
/// because the store makes the pointer escape.
#[test]
fn store_of_alloca_pointer_into_itself() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <llvm.ptr (0) () variadic = false> [] {
        ^entry():
        one = builtin.constant <builtin.integer <1: i32>> : builtin.integer i32;
        p = llvm.alloca [llvm.ptr (0) x one] : llvm.ptr (0);
        llvm.store *p <- p;
        v = llvm.load p : llvm.ptr (0);
        llvm.return v
      }
    "#;

    let (status, after) = run_mem2reg_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <llvm.ptr (0)() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            one_v0 = builtin.constant <builtin.integer <1: i32>> : builtin.integer i32 !1;
            p_v1 = llvm.alloca [llvm.ptr (0) x one_v0]  : llvm.ptr (0) !2;
            llvm.store *p_v1 <- p_v1  !3;
            v_v2 = llvm.load p_v1  : llvm.ptr (0) !4;
            llvm.return v_v2 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}
