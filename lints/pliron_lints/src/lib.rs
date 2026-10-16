//! Dylint lints for pliron and code built on it.

#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_data_structures;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

mod ir_borrow_with_context;
mod ir_ref_in_scrutinee;
mod utils;

dylint_linting::dylint_library!();

#[allow(clippy::no_mangle_with_rust_abi)]
#[unsafe(no_mangle)]
pub fn register_lints(sess: &rustc_session::Session, lint_store: &mut rustc_lint::LintStore) {
    dylint_linting::init_config(sess);
    lint_store.register_lints(&[
        ir_borrow_with_context::IR_BORROW_WITH_CONTEXT,
        ir_ref_in_scrutinee::IR_REF_IN_SCRUTINEE,
    ]);
    lint_store.register_late_lint_pass(Box::new(|_| {
        Box::new(ir_borrow_with_context::IrBorrowWithContext)
    }));
    lint_store.register_late_lint_pass(Box::new(|_| {
        Box::<ir_ref_in_scrutinee::IrRefInScrutinee>::default()
    }));
}

#[test]
fn ui() {
    dylint_testing::ui_test_example(env!("CARGO_PKG_NAME"), "ui");
}
