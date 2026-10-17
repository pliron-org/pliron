//! Test cases for the pliron lints. Each function states the expected result.

use std::cell::Ref;
use std::marker::PhantomData;

use pliron::basic_block::BasicBlock;
use pliron::builtin::ops::ConstantOp;
use pliron::builtin::types::IntegerType;
use pliron::context::{Context, Ptr};
use pliron::derive::{pliron_attr, pliron_op, pliron_type};
use pliron::linked_list::ContainsLinkedList;
use pliron::location::Location;
use pliron::op::Op;
use pliron::operation::Operation;
use pliron::parsable::{Parsable, ParseResult, StateStream};
use pliron::printable::{Printable, State};
use pliron::region::Region;
use pliron::r#type::{TypeHandle, TypedHandle};
use pliron::value::Value;

fn use_ctx(_ctx: &Context) {}

fn op_ptr() -> Ptr<Operation> {
    unimplemented!()
}

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

/// Empty `Printable` and `Parsable` implementations for the test types and attributes below.
macro_rules! empty_printable_parsable {
    ($name:ident, $parsed:ty) => {
        impl Printable for $name {
            fn fmt(
                &self,
                _ctx: &Context,
                _state: &State,
                _f: &mut core::fmt::Formatter<'_>,
            ) -> core::fmt::Result {
                Ok(())
            }
        }

        impl Parsable for $name {
            type Arg = ();
            type Parsed = $parsed;
            fn parse<'a>(
                _state_stream: &mut StateStream<'a>,
                _arg: Self::Arg,
            ) -> ParseResult<'a, Self::Parsed> {
                unimplemented!()
            }
        }
    };
}

// `ir_entity_in_type_or_attr` must report an error: the attribute holds a `Ptr<Operation>`.
#[pliron_attr(name = "test.op_ptr_attr", verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
struct OpPtrAttr {
    target: Ptr<Operation>,
}
empty_printable_parsable!(OpPtrAttr, Self);

// `ir_entity_in_type_or_attr` must report an error: each `Value` holds a `Ptr<Operation>` or a
// `Ptr<BasicBlock>`.
#[pliron_type(name = "test.values_type", verifier = "succ")]
#[derive(PartialEq, Eq, Debug, Hash)]
struct ValuesType {
    values: Vec<Value>,
}
empty_printable_parsable!(ValuesType, TypedHandle<Self>);

// `ir_entity_in_type_or_attr` must report an error: an `Op` holds its `Ptr<Operation>`.
#[pliron_attr(name = "test.op_attr", verifier = "succ")]
#[derive(PartialEq, Clone, Hash)]
struct OpAttr {
    constant: ConstantOp,
}

impl core::fmt::Debug for OpAttr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OpAttr")
    }
}
empty_printable_parsable!(OpAttr, Self);

// `ir_entity_in_type_or_attr` must report an error: the attribute holds a `Ptr<BasicBlock>`.
#[pliron_attr(name = "test.block_ptr_attr", verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
struct BlockPtrAttr {
    target: Ptr<BasicBlock>,
}
empty_printable_parsable!(BlockPtrAttr, Self);

// `ir_entity_in_type_or_attr` must report an error: the type holds a `Ptr<Region>` in an
// `Option`.
#[pliron_type(name = "test.region_ptr_type", verifier = "succ")]
#[derive(PartialEq, Eq, Debug, Hash)]
struct RegionPtrType {
    region: Option<Ptr<Region>>,
}
empty_printable_parsable!(RegionPtrType, TypedHandle<Self>);

// `ir_entity_in_type_or_attr` must not report an error: `PhantomData` holds nothing.
#[pliron_attr(name = "test.phantom_attr", verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
struct PhantomAttr {
    marker: PhantomData<Ptr<Operation>>,
}
empty_printable_parsable!(PhantomAttr, Self);

/// A marker that stores nothing of its type argument.
#[derive(PartialEq, Clone, Debug, Hash)]
struct Marker<T>(PhantomData<T>);

// `ir_entity_in_type_or_attr` must not report an error: `Marker` stores no `Ptr<Operation>`.
#[pliron_attr(name = "test.marker_attr", verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
struct MarkerAttr {
    marker: Marker<Ptr<Operation>>,
}
empty_printable_parsable!(MarkerAttr, Self);

/// A trait whose associated type gives the type of the stored data.
trait Storage {
    type Data: PartialEq + Clone + core::fmt::Debug + core::hash::Hash;
}

impl Storage for () {
    type Data = Ptr<Operation>;
}

/// Stores data of the type given by `T::Data`.
#[derive(PartialEq, Clone, Debug, Hash)]
struct Indirect<T: Storage> {
    data: T::Data,
}

// `ir_entity_in_type_or_attr` must report an error: `<() as Storage>::Data` is `Ptr<Operation>`.
#[pliron_attr(name = "test.indirect_attr", verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
struct IndirectAttr {
    indirect: Indirect<()>,
}
empty_printable_parsable!(IndirectAttr, Self);

// `ir_entity_in_type_or_attr` must not report an error: the struct is not a type or an attribute.
struct OpHolder {
    target: Ptr<Operation>,
}

#[pliron_op(name = "test.my_op", format, verifier = "succ")]
struct MyOp {}

impl MyOp {
    // `explicit_op_construction` must report an error and tell to use `Operation::new_op`.
    fn new_with_name(ctx: &mut Context) -> Self {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        MyOp { op }
    }

    // `self_op_construction` must warn and tell to use `Operation::new_op`.
    fn new_with_self(ctx: &mut Context) -> Self {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        Self { op }
    }

    // `explicit_op_construction` and `self_op_construction` must not report: the constructor
    // uses `Operation::new_op`.
    fn new_with_new_op(ctx: &mut Context) -> Self {
        Operation::new_op::<Self>(ctx, vec![], vec![], vec![], 0)
    }
}

// `explicit_op_construction` must report an error and tell to use `Operation::get_op`.
fn wrap_op(op: Ptr<Operation>) -> MyOp {
    MyOp { op }
}

// `explicit_op_construction` must not report an error: `Operation::get_op` checks the operation.
fn get_my_op(op: Ptr<Operation>, ctx: &Context) -> Option<MyOp> {
    Operation::get_op::<MyOp>(op, ctx)
}

/// Holds a type to print.
struct DispDirect(TypeHandle);

impl Printable for DispDirect {
    // `disp_in_printable` must warn and suggest `print(ctx, state)`.
    fn fmt(
        &self,
        ctx: &Context,
        state: &State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let _ = state;
        write!(f, "{}", self.0.disp(ctx))
    }
}

/// Holds a type to print.
struct DispInClosure(TypeHandle);

impl Printable for DispInClosure {
    // `disp_in_printable` must warn for the `disp` in the closure.
    fn fmt(
        &self,
        ctx: &Context,
        state: &State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let _ = state;
        let text = || self.0.disp(ctx).to_string();
        write!(f, "{}", text())
    }
}

/// Holds a type to print.
struct DispUnusedState(TypeHandle);

impl Printable for DispUnusedState {
    // `disp_in_printable` must warn, and the suggestion must use `_state`.
    fn fmt(
        &self,
        ctx: &Context,
        _state: &State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "{}", self.0.disp(ctx))
    }
}

/// Holds a type to print.
struct DispShadowedState(TypeHandle);

impl Printable for DispShadowedState {
    // `disp_in_printable` must warn without a code suggestion: `state` is bound again.
    fn fmt(
        &self,
        ctx: &Context,
        state: &State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let _ = state;
        let state = 0;
        write!(f, "{state} {}", self.0.disp(ctx))
    }
}

/// Holds a type to print.
struct PrintWithState(TypeHandle);

impl Printable for PrintWithState {
    // `disp_in_printable` must not warn: `print` passes the `State`.
    fn fmt(
        &self,
        ctx: &Context,
        state: &State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "{}", self.0.print(ctx, state))
    }
}

// `disp_in_printable` must not warn: the function is not in an `impl Printable`.
fn disp_outside_printable(ty: TypeHandle, ctx: &Context) -> String {
    ty.disp(ctx).to_string()
}

fn main() {
    let _ = OpHolder { target: op_ptr() }.target;
    let _ = (
        MyOp::new_with_name,
        MyOp::new_with_self,
        MyOp::new_with_new_op,
        wrap_op,
        get_my_op,
    );
    let _ = (
        disp_outside_printable,
        DispDirect,
        DispInClosure,
        DispUnusedState,
        DispShadowedState,
        PrintWithState,
    );
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
