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
extern crate rustc_trait_selection;

mod disp_in_printable;
mod explicit_op_construction;
mod ir_borrow_with_context;
mod ir_entity_in_type_or_attr;
mod ir_ref_in_scrutinee;
mod op_layout;
mod utils;

dylint_linting::dylint_library!();

#[allow(clippy::no_mangle_with_rust_abi)]
#[unsafe(no_mangle)]
pub fn register_lints(sess: &rustc_session::Session, lint_store: &mut rustc_lint::LintStore) {
    dylint_linting::init_config(sess);
    lint_store.register_lints(&[
        disp_in_printable::DISP_IN_PRINTABLE,
        explicit_op_construction::EXPLICIT_OP_CONSTRUCTION,
        explicit_op_construction::NEW_OP_OUTSIDE_CONSTRUCTOR,
        explicit_op_construction::SELF_OP_CONSTRUCTION,
        ir_borrow_with_context::IR_BORROW_WITH_CONTEXT,
        ir_entity_in_type_or_attr::IR_ENTITY_IN_TYPE_OR_ATTR,
        ir_ref_in_scrutinee::IR_REF_IN_SCRUTINEE,
        op_layout::OP_LAYOUT,
    ]);
    lint_store.register_late_lint_pass(Box::new(|_| Box::<op_layout::OpLayout>::default()));
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(disp_in_printable::DispInPrintable)));
    lint_store.register_late_lint_pass(Box::new(|_| {
        Box::new(explicit_op_construction::ExplicitOpConstruction)
    }));
    lint_store.register_late_lint_pass(Box::new(|_| {
        Box::new(ir_borrow_with_context::IrBorrowWithContext)
    }));
    lint_store.register_late_lint_pass(Box::new(|_| {
        Box::new(ir_entity_in_type_or_attr::IrEntityInTypeOrAttr)
    }));
    lint_store.register_late_lint_pass(Box::new(|_| {
        Box::<ir_ref_in_scrutinee::IrRefInScrutinee>::default()
    }));
}

#[test]
fn ui() {
    dylint_testing::ui_test_example(env!("CARGO_PKG_NAME"), "ui");
}
