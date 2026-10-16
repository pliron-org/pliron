//! Test cases for the pliron lints. Each function states the expected result.

use std::cell::Ref;

use pliron::basic_block::BasicBlock;
use pliron::builtin::ops::ConstantOp;
use pliron::builtin::types::IntegerType;
use pliron::context::{Context, Ptr};
use pliron::linked_list::ContainsLinkedList;
use pliron::location::Location;
use pliron::operation::Operation;

fn use_ctx(_ctx: &Context) {}

// `ir_borrow_with_context` must warn: `&Operation` and `&Context`.
fn borrow_and_context(op: &Operation, ctx: &Context) -> usize {
    use_ctx(ctx);
    op.get_num_results()
}

// `ir_borrow_with_context` must warn: `Ref<Operation>` and `&Context`.
fn ref_borrow_and_context(op: Ref<Operation>, ctx: &Context) -> usize {
    use_ctx(ctx);
    op.get_num_results()
}

// `ir_borrow_with_context` must not warn for the nested function: only its enclosing function
// calls it.
fn nested_borrow_and_context(op: Ptr<Operation>, ctx: &Context) -> usize {
    fn inner(op: &Operation, ctx: &Context) -> usize {
        use_ctx(ctx);
        op.get_num_results()
    }
    inner(&op.deref(ctx), ctx)
}

// `ir_borrow_with_context` must not warn: there is no `Context`.
fn borrow_only(op: &Operation) -> usize {
    op.get_num_results()
}

// `ir_borrow_with_context` must not warn: the IR object is passed as a `Ptr`.
fn ptr_and_context(op: Ptr<Operation>, ctx: &Context) -> usize {
    op.deref(ctx).get_num_results()
}

// `ir_ref_in_scrutinee` must warn: the body uses the `Context`.
fn for_with_ref(block: Ptr<BasicBlock>, ctx: &Context) {
    for _op in block.deref(ctx).iter(ctx) {
        use_ctx(ctx);
    }
}

// `ir_ref_in_scrutinee` must not warn: the `Ref` is dropped before the loop.
fn for_with_let(block: Ptr<BasicBlock>, ctx: &Context) {
    let ops = block.deref(ctx).iter(ctx);
    for _op in ops {
        use_ctx(ctx);
    }
}

// `ir_ref_in_scrutinee` must not warn: the body does not use the `Context`.
fn for_without_ctx(block: Ptr<BasicBlock>, ctx: &Context) -> usize {
    let mut count = 0;
    for _op in block.deref(ctx).iter(ctx) {
        count += 1;
    }
    count
}

// `ir_ref_in_scrutinee` must warn: the `while let` body uses the `Context`.
fn while_let_with_ref(mut op: Ptr<Operation>, ctx: &Context) {
    while let Some(block) = op.deref(ctx).get_parent_block() {
        use_ctx(ctx);
        let Some(parent_op) = block.deref(ctx).get_parent_op(ctx) else {
            break;
        };
        op = parent_op;
    }
}

// `ir_ref_in_scrutinee` must not warn: the body only uses the `Context` for shared `deref`s.
fn for_with_shared_deref(block: Ptr<BasicBlock>, ctx: &Context) -> usize {
    let mut count = 0;
    for op in block.deref(ctx).iter(ctx) {
        count += op.deref(ctx).get_num_results();
    }
    count
}

// `ir_ref_in_scrutinee` must warn: a shared `deref` of a `BasicBlock` conflicts with a
// `RefMut<BasicBlock>`.
fn for_with_ref_mut(block: Ptr<BasicBlock>, ctx: &Context) -> usize {
    let mut count = 0;
    for op in block.deref_mut(ctx).iter(ctx) {
        let parent = op.deref(ctx).get_parent_block().unwrap();
        count += parent.deref(ctx).get_num_arguments();
    }
    count
}

// `ir_ref_in_scrutinee` must not warn: a `deref_mut` of an `Operation` cannot conflict with a
// `Ref<BasicBlock>`.
fn for_with_other_entity_deref_mut(block: Ptr<BasicBlock>, ctx: &Context) {
    for op in block.deref(ctx).iter(ctx) {
        op.deref_mut(ctx).set_loc(Location::Unknown);
    }
}

// `ir_ref_in_scrutinee` must warn once: the `Ref` made by the attribute getter borrows the
// `Operation`. The `Ref` of the type must not give a warning.
fn if_let_with_attribute_ref(op: ConstantOp, ctx: &Context) -> u32 {
    if let Some(width) = op
        .get_value(ctx)
        .get_type(ctx)
        .deref(ctx)
        .downcast_ref::<IntegerType>()
        .map(|int_ty| int_ty.width())
    {
        use_ctx(ctx);
        return width;
    }
    0
}

// `ir_ref_in_scrutinee` must warn: `Ref::map` keeps the borrow of the `Operation`.
fn for_with_mapped_ref(op: Ptr<Operation>, ctx: &Context) {
    for _attr in Ref::map(op.deref(ctx), |op| &op.attributes).0.values() {
        use_ctx(ctx);
    }
}

// `ir_ref_in_scrutinee` must warn and suggest a `let`: `succs` does not borrow the `Ref`.
fn for_with_succs(block: Ptr<BasicBlock>, ctx: &Context) {
    for _succ in block.deref(ctx).succs(ctx) {
        use_ctx(ctx);
    }
}

// `ir_ref_in_scrutinee` must warn and suggest a `let`: `get_parent_block` does not borrow the
// `Ref`.
fn if_let_with_parent_block(op: Ptr<Operation>, ctx: &Context) {
    if let Some(_block) = op.deref(ctx).get_parent_block() {
        use_ctx(ctx);
    }
}

// `ir_ref_in_scrutinee` must warn and tell to collect the values: `results` borrows the `Ref`.
fn for_with_results(op: Ptr<Operation>, ctx: &Context) {
    for _result in op.deref(ctx).results() {
        use_ctx(ctx);
    }
}

// `ir_ref_in_scrutinee` must warn and tell to collect the values: `arguments` borrows the
// `Ref`.
fn for_with_arguments(block: Ptr<BasicBlock>, ctx: &Context) {
    for _arg in block.deref(ctx).arguments() {
        use_ctx(ctx);
    }
}

// `ir_ref_in_scrutinee` must not warn: the lint does not report the iterator over the operands.
fn for_with_operands(op: Ptr<Operation>, ctx: &Context) {
    for _operand in op.deref(ctx).operands() {
        use_ctx(ctx);
    }
}

// `ir_ref_in_scrutinee` must warn and tell to collect the values: `operands_as_uses` borrows
// the `Ref`.
fn for_with_operand_uses(op: Ptr<Operation>, ctx: &Context) {
    for _operand in op.deref(ctx).operands_as_uses() {
        use_ctx(ctx);
    }
}

// `ir_ref_in_scrutinee` must warn: the loop deletes the users of the operation while the `Ref`
// of the operation is alive.
fn for_uses_delete_user(op: Ptr<Operation>, ctx: &Context) {
    for r#use in op.deref(ctx).uses() {
        let user = r#use.user_op();
        Operation::drop_all_uses(user, ctx);
        user.unlink(ctx);
    }
}

// `ir_ref_in_scrutinee` must warn: each new use of a result changes the `Operation` that the
// `Ref` still borrows.
fn for_results_add_use(op: Ptr<Operation>, user: Ptr<Operation>, outs: &[u32], ctx: &Context) {
    for (_out, result) in outs.iter().zip(op.deref(ctx).results()) {
        Operation::push_operand(user, ctx, result);
    }
}

// `ir_ref_in_scrutinee` must warn and suggest a `let` for `succs`:
// the `iter` after it borrows the result of `succs`.
fn for_with_succs_iter(block: Ptr<BasicBlock>, ctx: &Context) {
    for _succ in block.deref(ctx).succs(ctx).iter() {
        use_ctx(ctx);
    }
}

// `ir_ref_in_scrutinee` must warn:
// a shared `deref` of the `Operation` conflicts with the `RefMut<Operation>`.
fn for_with_regions_ref_mut(op: Ptr<Operation>, ctx: &Context) -> usize {
    let mut count = 0;
    for _region in op.deref_mut(ctx).regions() {
        count += op.deref(ctx).num_regions();
    }
    count
}

fn main() {
    let _ = (
        borrow_and_context,
        ref_borrow_and_context,
        nested_borrow_and_context,
        borrow_only,
        ptr_and_context,
        for_with_ref,
        for_with_let,
        for_without_ctx,
        while_let_with_ref,
        for_with_shared_deref,
        for_with_ref_mut,
        for_with_other_entity_deref_mut,
        if_let_with_attribute_ref,
        for_with_mapped_ref,
        for_with_succs,
        if_let_with_parent_block,
        for_with_results,
        for_with_arguments,
        for_with_operands,
        for_with_operand_uses,
        for_uses_delete_user,
        for_results_add_use,
        for_with_succs_iter,
        for_with_regions_ref_mut,
    );
}
