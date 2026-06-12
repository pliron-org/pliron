//! Utilities for attaching / retrieving debug info to / from the IR.

use alloc::{string::ToString, vec::Vec};

use pliron::derive::{attr_interface_impl, pliron_attr};

use crate::{
    attribute::AttributeDict,
    basic_block::BasicBlock,
    builtin::{ATTR_KEY_DEBUG_INFO, attr_interfaces::OutlinedAttr},
    combine::{Parser, attempt, between, parser::char::spaces, sep_by, token},
    context::{Context, Ptr},
    deps::hash::hash_map::Entry,
    identifier::Identifier,
    operation::Operation,
    parsable::{Parsable, ParseResult, StateStream},
    printable::{self, Printable},
    utils::vec_exns::VecExtns,
};

#[pliron_attr(name = "builtin.debug_info", verifier = "succ")]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
struct DebugInfoAttr {
    names: Vec<Option<Identifier>>,
}

impl DebugInfoAttr {
    fn get_name(&self, idx: usize) -> Option<Identifier> {
        self.names.get(idx).cloned().flatten()
    }

    fn set_name(&mut self, idx: usize, name: Option<Identifier>) {
        self.names.grow_to(idx + 1, |_| None);
        *self.names.get_mut(idx).unwrap() = name;
    }

    fn insert_name(&mut self, idx: usize, name: Option<Identifier>) {
        self.names.grow_to(idx, |_| None);
        self.names.insert(idx, name);
    }

    fn remove_name(&mut self, idx: usize) {
        if idx < self.names.len() {
            self.names.remove(idx);
        }
    }
}

#[attr_interface_impl]
impl OutlinedAttr for DebugInfoAttr {}

impl Printable for DebugInfoAttr {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(
            f,
            "[{}]",
            self.names
                .iter()
                .map(|name| match name {
                    Some(name) => name.disp(ctx).to_string(),
                    None => "?".to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl Parsable for DebugInfoAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        between(
            token('[').skip(spaces()),
            spaces().with(token(']')),
            sep_by(
                attempt(token('?').map(|_| None)).or(Identifier::parser(()).map(Some)),
                token(',').skip(spaces()),
            ),
        )
        .map(|names| DebugInfoAttr { names })
        .parse_stream(state_stream)
        .into()
    }
}

fn set_name_in_attr_map(attributes: &mut AttributeDict, idx: usize, name: Option<Identifier>) {
    match attributes.0.entry(ATTR_KEY_DEBUG_INFO.clone()) {
        Entry::Occupied(mut occupied) => {
            let debug_info = occupied
                .get_mut()
                .downcast_mut::<DebugInfoAttr>()
                .expect("Existing attribute entry for debug info incorrect");
            let name_is_none = name.is_none();
            debug_info.set_name(idx, name);
            // If the debug info attribute has all None entries, remove it from the map.
            if name_is_none && debug_info.names.iter().all(|name| name.is_none()) {
                occupied.remove();
            }
        }
        Entry::Vacant(vacant) => {
            // Only insert a new debug info attribute if there's actually a name to set.
            if name.is_some() {
                let mut debug_info = DebugInfoAttr::default();
                debug_info.set_name(idx, name);
                vacant.insert(debug_info.into());
            }
        }
    }
}

fn insert_name_in_attr_map(attributes: &mut AttributeDict, idx: usize, name: Option<Identifier>) {
    match attributes.0.entry(ATTR_KEY_DEBUG_INFO.clone()) {
        Entry::Occupied(mut occupied) => {
            let debug_info = occupied
                .get_mut()
                .downcast_mut::<DebugInfoAttr>()
                .expect("Existing attribute entry for debug info incorrect");
            let name_is_none = name.is_none();
            debug_info.insert_name(idx, name);
            // If the debug info attribute has all None entries, remove it from the map.
            if name_is_none && debug_info.names.iter().all(|name| name.is_none()) {
                occupied.remove();
            }
        }
        Entry::Vacant(vacant) => {
            // Only insert a new debug info attribute if there's actually a name to set.
            if name.is_some() {
                let mut debug_info = DebugInfoAttr::default();
                debug_info.insert_name(idx, name);
                vacant.insert(debug_info.into());
            }
        }
    }
}

fn remove_name_from_attr_map(attributes: &mut AttributeDict, idx: usize) {
    if let Entry::Occupied(mut occupied) = attributes.0.entry(ATTR_KEY_DEBUG_INFO.clone()) {
        let debug_info = occupied
            .get_mut()
            .downcast_mut::<DebugInfoAttr>()
            .expect("Existing attribute entry for debug info incorrect");
        debug_info.remove_name(idx);
        // If the debug info attribute has no more names, remove it from the map.
        if debug_info.names.iter().all(|name| name.is_none()) {
            occupied.remove();
        }
    }
}

fn get_name_from_attr_map(attributes: &AttributeDict, idx: usize) -> Option<Identifier> {
    attributes
        .get::<DebugInfoAttr>(&ATTR_KEY_DEBUG_INFO)
        .and_then(|debug_info| debug_info.get_name(idx))
}

/// Set the name for a result in an [Operation].
/// Panics if the given `res_idx` is out of range.
pub fn set_operation_result_name(
    ctx: &Context,
    op: Ptr<Operation>,
    res_idx: usize,
    name: Option<Identifier>,
) {
    let op = &mut *op.deref_mut(ctx);
    let num_results = op.get_num_results();
    assert!(res_idx < num_results);

    set_name_in_attr_map(&mut op.attributes, res_idx, name);
}

/// Insert a name for a result in an [Operation] at the given index,
/// shifting existing names at that index and beyond to the right.
/// Panics if the given `res_idx` is out of range (i.e., `res_idx > num_results`).
pub fn insert_operation_result_name(
    ctx: &Context,
    op: Ptr<Operation>,
    res_idx: usize,
    name: Option<Identifier>,
) {
    let op = &mut *op.deref_mut(ctx);
    let num_results = op.get_num_results();
    assert!(res_idx <= num_results);

    insert_name_in_attr_map(&mut op.attributes, res_idx, name);
}

/// Remove the name for a result in an [Operation] at the given index,
/// shifting existing names at beyond that index to the left.
/// Panics if the given `res_idx` is out of range.
pub fn remove_operation_result_name(ctx: &Context, op: Ptr<Operation>, res_idx: usize) {
    let op = &mut *op.deref_mut(ctx);
    let num_results = op.get_num_results();
    assert!(res_idx < num_results);

    remove_name_from_attr_map(&mut op.attributes, res_idx);
}

/// Get name for a result in an [Operation].
pub fn get_operation_result_name(
    ctx: &Context,
    op: Ptr<Operation>,
    res_idx: usize,
) -> Option<Identifier> {
    let op = &*op.deref(ctx);
    get_name_from_attr_map(&op.attributes, res_idx)
}

/// Set the name for an argument in a [BasicBlock].
/// Panics if the given `arg_idx` is out of range.
pub fn set_block_arg_name(
    ctx: &Context,
    block: Ptr<BasicBlock>,
    arg_idx: usize,
    name: Option<Identifier>,
) {
    let block = &mut *block.deref_mut(ctx);
    let num_args = block.get_num_arguments();
    assert!(arg_idx < num_args);

    set_name_in_attr_map(&mut block.attributes, arg_idx, name);
}

/// Insert a name for an argument in a [BasicBlock] at the given index,
/// shifting existing names at that index and beyond to the right.
/// Panics if the given `arg_idx` is out of range (i.e., `arg_idx > num_args`).
pub fn insert_block_arg_name(
    ctx: &Context,
    block: Ptr<BasicBlock>,
    arg_idx: usize,
    name: Option<Identifier>,
) {
    let block = &mut *block.deref_mut(ctx);
    let num_args = block.get_num_arguments();
    assert!(arg_idx <= num_args);

    insert_name_in_attr_map(&mut block.attributes, arg_idx, name);
}

/// Remove the name for an argument in a [BasicBlock] at the given index,
/// shifting existing names at beyond that index to the left.
/// Panics if the given `arg_idx` is out of range.
pub fn remove_block_arg_name(ctx: &Context, block: Ptr<BasicBlock>, arg_idx: usize) {
    let block = &mut *block.deref_mut(ctx);
    let num_args = block.get_num_arguments();
    assert!(arg_idx < num_args);

    remove_name_from_attr_map(&mut block.attributes, arg_idx);
}

/// Get name for an argument in a [BasicBlock].
pub fn get_block_arg_name(
    ctx: &Context,
    block: Ptr<BasicBlock>,
    arg_idx: usize,
) -> Option<Identifier> {
    let block = &*block.deref(ctx);
    get_name_from_attr_map(&block.attributes, arg_idx)
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use pliron::derive::pliron_op;

    use crate::{
        basic_block::BasicBlock,
        builtin::{
            op_interfaces::{NOpdsInterface, NResultsInterface, OneResultInterface},
            types::{IntegerType, Signedness},
        },
        context::Context,
        debug_info::{
            get_block_arg_name, get_operation_result_name, insert_block_arg_name,
            insert_operation_result_name, remove_block_arg_name, remove_operation_result_name,
            set_block_arg_name, set_operation_result_name,
        },
        op::Op,
        operation::{Operation, verify_operation},
        result::Result,
    };

    #[pliron_op(
        name = "test.zero",
        format,
        interfaces = [
            OneResultInterface, NResultsInterface<1>, NOpdsInterface<0>
        ],
        verifier = "succ",
    )]
    struct ZeroOp;
    impl ZeroOp {
        pub fn new(ctx: &mut Context) -> Self {
            let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
            ZeroOp {
                op: Operation::new(
                    ctx,
                    Self::get_concrete_op_info(),
                    vec![i64_ty.into()],
                    vec![],
                    vec![],
                    0,
                ),
            }
        }
    }

    #[test]
    fn test_op_result_name() -> Result<()> {
        let mut ctx = Context::new();
        let cop = ZeroOp::new(&mut ctx);
        let op = cop.get_operation();
        set_operation_result_name(&ctx, op, 0, Some("foo".try_into().unwrap()));
        assert_eq!(
            get_operation_result_name(&ctx, op, 0).unwrap(),
            "foo".try_into().unwrap()
        );
        verify_operation(op, &ctx)?;
        Ok(())
    }

    #[test]
    fn test_block_arg_name() -> Result<()> {
        let mut ctx = Context::new();
        let i64_ty = IntegerType::get(&mut ctx, 64, Signedness::Signed);
        let block = BasicBlock::new(
            &mut ctx,
            Some("entry".try_into().unwrap()),
            vec![i64_ty.into()],
        );
        set_block_arg_name(&ctx, block, 0, Some("foo".try_into().unwrap()));
        assert!(get_block_arg_name(&ctx, block, 0).unwrap() == "foo".try_into().unwrap());
        Ok(())
    }

    #[test]
    fn test_op_result_name_insert_remove_shift() {
        let mut ctx = Context::new();
        let i64_ty = IntegerType::get(&mut ctx, 64, Signedness::Signed);
        let op = Operation::new(
            &mut ctx,
            ZeroOp::get_concrete_op_info(),
            vec![i64_ty.into(), i64_ty.into(), i64_ty.into()],
            vec![],
            vec![],
            0,
        );

        set_operation_result_name(&ctx, op, 0, Some("r0".try_into().unwrap()));
        set_operation_result_name(&ctx, op, 1, Some("r1".try_into().unwrap()));
        assert_eq!(
            get_operation_result_name(&ctx, op, 0),
            Some("r0".try_into().unwrap())
        );
        assert_eq!(
            get_operation_result_name(&ctx, op, 1),
            Some("r1".try_into().unwrap())
        );
        assert_eq!(get_operation_result_name(&ctx, op, 2), None);

        // Insert/remove at end should not affect earlier indices.
        insert_operation_result_name(&ctx, op, 2, Some("tail".try_into().unwrap()));
        assert_eq!(
            get_operation_result_name(&ctx, op, 0),
            Some("r0".try_into().unwrap())
        );
        assert_eq!(
            get_operation_result_name(&ctx, op, 1),
            Some("r1".try_into().unwrap())
        );
        assert_eq!(
            get_operation_result_name(&ctx, op, 2),
            Some("tail".try_into().unwrap())
        );
        remove_operation_result_name(&ctx, op, 2);
        assert_eq!(
            get_operation_result_name(&ctx, op, 0),
            Some("r0".try_into().unwrap())
        );
        assert_eq!(
            get_operation_result_name(&ctx, op, 1),
            Some("r1".try_into().unwrap())
        );
        assert_eq!(get_operation_result_name(&ctx, op, 2), None);

        // Insert a placeholder at front, shifting r0 name to index 1.
        insert_operation_result_name(&ctx, op, 0, None);
        assert_eq!(get_operation_result_name(&ctx, op, 0), None);
        assert_eq!(
            get_operation_result_name(&ctx, op, 1),
            Some("r0".try_into().unwrap())
        );
        assert_eq!(
            get_operation_result_name(&ctx, op, 2),
            Some("r1".try_into().unwrap())
        );

        // Insert a named result at the front.
        insert_operation_result_name(&ctx, op, 0, Some("ins".try_into().unwrap()));
        assert_eq!(
            get_operation_result_name(&ctx, op, 0),
            Some("ins".try_into().unwrap())
        );
        assert_eq!(get_operation_result_name(&ctx, op, 1), None);
        assert_eq!(
            get_operation_result_name(&ctx, op, 2),
            Some("r0".try_into().unwrap())
        );
        assert_eq!(
            get_operation_result_name(&ctx, op, 3),
            Some("r1".try_into().unwrap())
        );

        // Remove front name; prior index 1 becomes 0 and index 2 becomes 1.
        remove_operation_result_name(&ctx, op, 0);
        assert_eq!(get_operation_result_name(&ctx, op, 0), None);
        assert_eq!(
            get_operation_result_name(&ctx, op, 1),
            Some("r0".try_into().unwrap())
        );
        assert_eq!(
            get_operation_result_name(&ctx, op, 2),
            Some("r1".try_into().unwrap())
        );

        // Remove the placeholder, moving r0 back to index 0.
        remove_operation_result_name(&ctx, op, 0);
        assert_eq!(
            get_operation_result_name(&ctx, op, 0),
            Some("r0".try_into().unwrap())
        );
        assert_eq!(
            get_operation_result_name(&ctx, op, 1),
            Some("r1".try_into().unwrap())
        );
    }

    #[test]
    fn test_block_arg_name_insert_remove_shift() {
        let mut ctx = Context::new();
        let i64_ty = IntegerType::get(&mut ctx, 64, Signedness::Signed);
        let block = BasicBlock::new(
            &mut ctx,
            Some("entry".try_into().unwrap()),
            vec![i64_ty.into(), i64_ty.into(), i64_ty.into()],
        );

        set_block_arg_name(&ctx, block, 0, Some("a0".try_into().unwrap()));
        set_block_arg_name(&ctx, block, 1, Some("a1".try_into().unwrap()));
        assert_eq!(
            get_block_arg_name(&ctx, block, 0),
            Some("a0".try_into().unwrap())
        );
        assert_eq!(
            get_block_arg_name(&ctx, block, 1),
            Some("a1".try_into().unwrap())
        );
        assert_eq!(get_block_arg_name(&ctx, block, 2), None);

        // Insert/remove at end should not affect earlier indices.
        insert_block_arg_name(&ctx, block, 2, Some("tail".try_into().unwrap()));
        assert_eq!(
            get_block_arg_name(&ctx, block, 0),
            Some("a0".try_into().unwrap())
        );
        assert_eq!(
            get_block_arg_name(&ctx, block, 1),
            Some("a1".try_into().unwrap())
        );
        assert_eq!(
            get_block_arg_name(&ctx, block, 2),
            Some("tail".try_into().unwrap())
        );
        remove_block_arg_name(&ctx, block, 2);
        assert_eq!(
            get_block_arg_name(&ctx, block, 0),
            Some("a0".try_into().unwrap())
        );
        assert_eq!(
            get_block_arg_name(&ctx, block, 1),
            Some("a1".try_into().unwrap())
        );
        assert_eq!(get_block_arg_name(&ctx, block, 2), None);

        // Insert a placeholder at front, shifting a0 to index 1.
        insert_block_arg_name(&ctx, block, 0, None);
        assert_eq!(get_block_arg_name(&ctx, block, 0), None);
        assert_eq!(
            get_block_arg_name(&ctx, block, 1),
            Some("a0".try_into().unwrap())
        );
        assert_eq!(
            get_block_arg_name(&ctx, block, 2),
            Some("a1".try_into().unwrap())
        );

        // Insert a named arg at the front.
        insert_block_arg_name(&ctx, block, 0, Some("ins".try_into().unwrap()));
        assert_eq!(
            get_block_arg_name(&ctx, block, 0),
            Some("ins".try_into().unwrap())
        );
        assert_eq!(get_block_arg_name(&ctx, block, 1), None);
        assert_eq!(
            get_block_arg_name(&ctx, block, 2),
            Some("a0".try_into().unwrap())
        );
        assert_eq!(
            get_block_arg_name(&ctx, block, 3),
            Some("a1".try_into().unwrap())
        );

        // Remove front name; prior index 1 becomes 0 and index 2 becomes 1.
        remove_block_arg_name(&ctx, block, 0);
        assert_eq!(get_block_arg_name(&ctx, block, 0), None);
        assert_eq!(
            get_block_arg_name(&ctx, block, 1),
            Some("a0".try_into().unwrap())
        );
        assert_eq!(
            get_block_arg_name(&ctx, block, 2),
            Some("a1".try_into().unwrap())
        );

        // Remove placeholder, moving a0 back to index 0.
        remove_block_arg_name(&ctx, block, 0);
        assert_eq!(
            get_block_arg_name(&ctx, block, 0),
            Some("a0".try_into().unwrap())
        );
        assert_eq!(
            get_block_arg_name(&ctx, block, 1),
            Some("a1".try_into().unwrap())
        );
    }
}
