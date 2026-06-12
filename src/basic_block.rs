//! A [BasicBlock] is a list of [Operation]s.

use alloc::{format, string::String, vec, vec::Vec};
use combine::{
    optional,
    parser::{Parser, char::spaces},
    sep_by, token,
};
use thiserror::Error;

use crate::{
    attribute::AttributeDict,
    builtin::op_interfaces::{IsTerminatorInterface, NoTerminatorInterface},
    common_traits::{Named, RcShare, Verify},
    context::{Arena, Context, Ptr, private::ArenaObj},
    debug_info::{self, set_block_arg_name},
    identifier::Identifier,
    indented_block,
    irfmt::{
        outlined::{preprint_outline_block, register_block_for_outline},
        parsers::{delimited_list_parser, location, spaced, type_parser},
        printers::iter_with_sep,
    },
    linked_list::{ContainsLinkedList, LinkedList, private},
    location::{Located, Location},
    op::op_impls,
    operation::{DefUseVerifyErr, OpDbg, Operation, OperationParserConfig},
    parsable::{self, IntoParseResult, Parsable, ParseResult},
    printable::{self, ListSeparator, Printable, indented_nl},
    region::Region,
    result::Result,
    r#type::{TypeObj, Typed},
    utils::vec_exns::VecExtns,
    value::{DefNode, DefiningEntity, Value},
    verify_err, verify_error,
};

/// Argument to a [BasicBlock]
pub(crate) struct BlockArgument {
    /// The def containing the list of this argument's uses.
    pub(crate) def: DefNode<Value>,
    /// Unique ID of this value in [Context].
    pub(crate) val_uid: u64,
    /// The [Type](crate::type::Type) of this argument.
    pub(crate) ty: Ptr<TypeObj>,
}

impl BlockArgument {
    /// Create a new block argument with the given type and a fresh value UID.
    pub(crate) fn new(ctx: &Context, ty: Ptr<TypeObj>) -> Self {
        Self {
            def: DefNode::new(),
            val_uid: ctx.get_new_value_uid(),
            ty,
        }
    }

    /// Get the [Type](crate::type::Type) of this argument.
    pub(crate) fn get_type(&self, _ctx: &Context) -> Ptr<TypeObj> {
        self.ty
    }

    /// Set the [Type](crate::type::Type) of this argument.
    pub(crate) fn set_type(&mut self, _ctx: &Context, ty: Ptr<TypeObj>) {
        self.ty = ty;
    }

    /// Build a [Value] corresponding to this argument.
    pub(crate) fn as_value(&self, block: Ptr<BasicBlock>) -> Value {
        Value {
            defining_entity: DefiningEntity::Block(block),
            val_uid: self.val_uid,
        }
    }
}

impl Typed for BlockArgument {
    fn get_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        self.get_type(ctx)
    }
}

/// [Operation]s contained in this [BasicBlock]
#[derive(Default)]
pub struct OpsInBlock {
    first: Option<Ptr<Operation>>,
    last: Option<Ptr<Operation>>,
}

/// Links a [BasicBlock] with other blocks and the container [Region].
#[derive(Default)]
struct RegionLinks {
    /// Parent region of this block.
    parent_region: Option<Ptr<Region>>,
    /// The next block in the region's list of block.
    next_block: Option<Ptr<BasicBlock>>,
    /// The previous block in the region's list of blocks.
    prev_block: Option<Ptr<BasicBlock>>,
}

/// A basic block contains a list of [Operation]s. It may have arguments.
pub struct BasicBlock {
    pub(crate) self_ptr: Ptr<BasicBlock>,
    pub(crate) label: Option<Identifier>,
    pub(crate) ops_list: OpsInBlock,
    pub(crate) args: Vec<BlockArgument>,
    pub(crate) preds: DefNode<Ptr<BasicBlock>>,
    /// Links to the parent [Region] and
    /// previous and next [BasicBlock]s in the block.
    region_links: RegionLinks,
    /// A dictionary of attributes.
    pub attributes: AttributeDict,
    loc: Location,
}

impl Named for BasicBlock {
    fn given_name(&self, _ctx: &Context) -> Option<Identifier> {
        self.label.clone()
    }
    fn id(&self, _ctx: &Context) -> Identifier {
        self.self_ptr.make_name("block")
    }
}

impl BasicBlock {
    /// Create a new Basic Block.
    pub fn new(
        ctx: &mut Context,
        label: Option<Identifier>,
        arg_types: Vec<Ptr<TypeObj>>,
    ) -> Ptr<BasicBlock> {
        let f = |self_ptr: Ptr<BasicBlock>| BasicBlock {
            self_ptr,
            label,
            args: vec![],
            ops_list: OpsInBlock::default(),
            preds: DefNode::new(),
            region_links: RegionLinks::default(),
            attributes: AttributeDict::default(),
            loc: Location::Unknown,
        };
        let newblock = Self::alloc(ctx, f);
        // Let's update the args of the new block. Easier to do it here than during creation.
        let args = arg_types
            .into_iter()
            .map(|ty| BlockArgument::new(ctx, ty))
            .collect();
        newblock.deref_mut(ctx).args = args;
        // We're done.
        newblock
    }

    /// Set the block's label.
    /// This corresponds to the [given_name](Named::given_name) of this block.
    pub fn set_label(&mut self, _ctx: &Context, label: Option<Identifier>) {
        self.label = label;
    }

    /// Get parent region.
    pub fn get_parent_region(&self) -> Option<Ptr<Region>> {
        self.region_links.parent_region
    }

    /// Get parent operation.
    pub fn get_parent_op(&self, ctx: &Context) -> Option<Ptr<Operation>> {
        self.get_parent_region()
            .map(|region| region.deref(ctx).get_parent_op())
    }

    /// Get parent block.
    pub fn get_parent_block(&self, ctx: &Context) -> Option<Ptr<BasicBlock>> {
        self.get_parent_op(ctx)
            .and_then(|op| op.deref(ctx).get_parent_block())
    }

    /// Get idx'th argument as a Value. Panics on invalid index.
    pub fn get_argument(&self, arg_idx: usize) -> Value {
        self.args[arg_idx].as_value(self.self_ptr)
    }

    /// Get an iterator over the arguments
    pub fn arguments(&self) -> impl Iterator<Item = Value> + '_ {
        self.args.iter().map(|arg| arg.as_value(self.self_ptr))
    }

    /// Add an argument to the end of the argument list, returning its index.
    pub fn push_argument(block: Ptr<BasicBlock>, ctx: &Context, ty: Ptr<TypeObj>) -> usize {
        let new_block_arg = BlockArgument::new(ctx, ty);
        block.deref_mut(ctx).args.push_back(new_block_arg)
    }

    /// Remove the last argument. Panics if there are no arguments or if the argument has uses.
    /// Any [Value] referring to the removed argument is invalidated.
    pub fn pop_argument(block: Ptr<BasicBlock>, ctx: &Context) {
        let len = block.deref(ctx).args.len();
        assert!(
            len > 0,
            "Can't pop argument from block {} with no arguments",
            block.deref(ctx).unique_name(ctx)
        );
        Self::remove_argument(block, ctx, len - 1);
    }

    /// Insert a new argument at `arg_idx`, shifting existing arguments, from `arg_idx`, to the right.
    /// Panics on invalid index.
    pub fn insert_argument(
        block: Ptr<BasicBlock>,
        ctx: &Context,
        arg_idx: usize,
        ty: Ptr<TypeObj>,
    ) {
        let new_block_arg = BlockArgument::new(ctx, ty);
        block.deref_mut(ctx).args.insert(arg_idx, new_block_arg);
        debug_info::insert_block_arg_name(ctx, block, arg_idx, None);
    }

    /// Remove the argument at `arg_idx`, shifting subsequent arguments to the left.
    /// Panics on invalid index or if the argument has uses.
    /// Any [Value] referring to the removed argument is invalidated.
    pub fn remove_argument(block: Ptr<BasicBlock>, ctx: &Context, arg_idx: usize) {
        let value = block.deref(ctx).get_argument(arg_idx);
        assert!(
            !value.is_used(ctx),
            "Can't remove argument {} from block {} with uses",
            arg_idx,
            block.deref(ctx).unique_name(ctx)
        );
        debug_info::remove_block_arg_name(ctx, block, arg_idx);
        block.deref_mut(ctx).args.remove(arg_idx);
    }

    /// Get the number of arguments.
    pub fn get_num_arguments(&self) -> usize {
        self.args.len()
    }

    /// Get all successors of this block.
    pub fn succs(&self, ctx: &Context) -> Vec<Ptr<BasicBlock>> {
        self.get_terminator(ctx)
            .map(|term| term.deref(ctx).successors().collect())
            .unwrap_or_default()
    }

    /// Does this block have any successors?
    pub fn has_succ(&self, ctx: &Context) -> bool {
        self.get_terminator(ctx)
            .map(|term| term.deref(ctx).get_num_successors() > 0)
            .unwrap_or(false)
    }

    /// Is `succ` a successor of this block?
    /// O(n) in the number of successors.
    pub fn is_succ(&self, ctx: &Context, succ: Ptr<BasicBlock>) -> bool {
        self.get_terminator(ctx)
            .is_some_and(|term| term.deref(ctx).successors().any(|s| s == succ))
    }

    /// Get the `i`-th successor of this block.
    /// Panics if there is no terminator or if `i` is out of bounds.
    pub fn get_succ(&self, ctx: &Context, i: usize) -> Ptr<BasicBlock> {
        self.get_terminator(ctx)
            .expect("Can't get successor of block with no terminator")
            .deref(ctx)
            .get_successor(i)
    }

    /// Get the number of successors of this block. Returns 0 if there is no terminator.
    pub fn num_succ(&self, ctx: &Context) -> usize {
        self.get_terminator(ctx)
            .map(|term| term.deref(ctx).get_num_successors())
            .unwrap_or(0)
    }

    /// Get the block terminator, if one exists.
    pub fn get_terminator(&self, ctx: &Context) -> Option<Ptr<Operation>> {
        let last_opr = self.get_tail()?;
        let last_op = Operation::get_op_dyn(last_opr, ctx);
        op_impls::<dyn IsTerminatorInterface>(last_op.as_ref()).then_some(last_opr)
    }

    /// Drop all uses that this block holds.
    pub fn drop_all_uses(ptr: Ptr<Self>, ctx: &Context) {
        let ops: Vec<_> = ptr.deref(ctx).iter(ctx).collect();
        for op in ops {
            Operation::drop_all_uses(op, ctx);
        }
    }

    /// Unlink and deallocate this block and everything that it contains.
    /// Panics if there are any uses outside the block.
    pub fn erase(ptr: Ptr<Self>, ctx: &mut Context) {
        Self::drop_all_uses(ptr, ctx);
        assert!(
            !ptr.has_pred(ctx),
            "BasicBlock {} with predecessor {} being erased",
            ptr.deref(ctx).unique_name(ctx),
            *ptr.preds(ctx).first().unwrap().deref(ctx).unique_name(ctx)
        );
        if let Some(op) = ptr.deref(ctx).iter(ctx).find(|op| op.deref(ctx).has_use()) {
            panic!(
                "Attemping to erase block {} which contains {} with use outside the block",
                ptr.deref(ctx).unique_name(ctx),
                OpDbg { op, ctx }
            );
        }
        if ptr.is_linked(ctx) {
            ptr.unlink(ctx);
        }
        ArenaObj::dealloc(ptr, ctx);
    }
}

impl Located for BasicBlock {
    fn loc(&self) -> Location {
        self.loc.clone()
    }

    fn set_loc(&mut self, loc: Location) {
        self.loc = loc;
    }
}

impl private::ContainsLinkedList<Operation> for BasicBlock {
    fn set_head(&mut self, head: Option<Ptr<Operation>>) {
        self.ops_list.first = head;
    }

    fn set_tail(&mut self, tail: Option<Ptr<Operation>>) {
        self.ops_list.last = tail;
    }
}

impl ContainsLinkedList<Operation> for BasicBlock {
    fn get_head(&self) -> Option<Ptr<Operation>> {
        self.ops_list.first
    }

    fn get_tail(&self) -> Option<Ptr<Operation>> {
        self.ops_list.last
    }
}

impl PartialEq for BasicBlock {
    fn eq(&self, other: &Self) -> bool {
        self.self_ptr == other.self_ptr
    }
}

impl private::LinkedList for BasicBlock {
    type ContainerType = Region;
    fn set_next(&mut self, next: Option<Ptr<Self>>) {
        self.region_links.next_block = next;
    }
    fn set_prev(&mut self, prev: Option<Ptr<Self>>) {
        self.region_links.prev_block = prev;
    }
    fn set_container(&mut self, container: Option<Ptr<Self::ContainerType>>) {
        self.region_links.parent_region = container;
    }
}

impl LinkedList for BasicBlock {
    fn get_next(&self) -> Option<Ptr<Self>> {
        self.region_links.next_block
    }
    fn get_prev(&self) -> Option<Ptr<Self>> {
        self.region_links.prev_block
    }
    fn get_container(&self) -> Option<Ptr<Self::ContainerType>> {
        self.region_links.parent_region
    }
}

impl ArenaObj for BasicBlock {
    fn get_arena(ctx: &Context) -> &Arena<Self> {
        &ctx.basic_blocks
    }
    fn get_arena_mut(ctx: &mut Context) -> &mut Arena<Self> {
        &mut ctx.basic_blocks
    }
    fn dealloc_sub_objects(ptr: Ptr<Self>, ctx: &mut Context) {
        let ops: Vec<_> = ptr.deref_mut(ctx).iter(ctx).collect();
        for op in ops {
            ArenaObj::dealloc(op, ctx);
        }
    }
    fn get_self_ptr(&self, _ctx: &Context) -> Ptr<Self> {
        self.self_ptr
    }
}

impl Verify for BasicBlock {
    fn verify(&self, ctx: &Context) -> Result<()> {
        // Ensure that the block has a terminator
        // (unless the enclosing [Op] is marked [NoTerminatorInterface].
        let label: String = self.unique_name(ctx).into();
        let parent_op = self.get_parent_op(ctx).ok_or_else(|| {
            verify_error!(self.loc(), BasicBlockVerifyErr::NoParent(label.clone()))
        })?;
        let parent_op = Operation::get_op_dyn(parent_op, ctx);
        if !op_impls::<dyn NoTerminatorInterface>(parent_op.as_ref())
            && self.get_terminator(ctx).is_none()
        {
            verify_err!(self.loc(), BasicBlockVerifyErr::MissingTerminator(label))?;
        }
        // Check that every use is of this block.
        for r#use in self.self_ptr.uses(ctx) {
            if r#use.get_def(ctx) != self.self_ptr {
                verify_err!(self.loc(), DefUseVerifyErr::OperandNotUseOfDef)?;
            }
        }
        // Check that every predecessor points back to this block.
        // (This is almost a repetition of the previous check).
        for pred in self.self_ptr.preds(ctx) {
            if !pred.deref(ctx).is_succ(ctx, self.self_ptr) {
                verify_err!(self.loc(), DefUseVerifyErr::OperandNotUseOfDef)?;
            }
        }
        self.args
            .iter()
            .try_for_each(|arg| arg.as_value(self.self_ptr).verify(ctx))?;
        self.iter(ctx).try_for_each(|op| op.deref(ctx).verify(ctx))
    }
}

/// Error indicating that a basic block is missing a terminator.
#[derive(Debug, Error)]

pub enum BasicBlockVerifyErr {
    #[error("Basic block \"{0}\" is missing a terminator")]
    MissingTerminator(String),
    #[error("Basic block \"{0}\" has a terminator that is not the last operation in the block")]
    TerminatorNotLast(String),
    #[error("Basic block \"{0}\" has no parent operation")]
    NoParent(String),
}

impl Printable for BasicBlock {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(
            f,
            "^{}({})",
            self.unique_name(ctx),
            iter_with_sep(
                self.args.iter().map(|arg| {
                    format!(
                        "{}: {}",
                        arg.as_value(self.self_ptr).disp(ctx),
                        arg.get_type(ctx).disp(ctx)
                    )
                }),
                ListSeparator::CharSpace(',')
            )
            .print(ctx, state),
        )?;

        // Print non-outlined attributes inline.
        let inline_attrs = self.attributes.clone_skip_outlined();
        if !inline_attrs.0.is_empty() {
            write!(f, " ")?;
            inline_attrs.fmt(ctx, state, f)?;
        }

        // Print the outline index (!N) if there is a location or any outlined attributes.
        preprint_outline_block(ctx, self.self_ptr, state.share(), f)?;

        write!(f, ":")?;

        indented_block!(state, {
            write!(
                f,
                "{}{}",
                indented_nl(state),
                iter_with_sep(self.iter(ctx), ListSeparator::CharNewline(';')).print(ctx, state),
            )?;
        });

        Ok(())
    }
}

impl Parsable for BasicBlock {
    type Arg = ();
    type Parsed = Ptr<BasicBlock>;

    //  A basic block is
    // ```
    //  label(arg_1: type_1, ..., arg_n: type_n) [optional_inline_attrs] [!N]:
    //    op_1;
    //    ... ;
    //    op_n
    // ```
    fn parse<'a>(
        state_stream: &mut parsable::StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let loc = state_stream.loc();

        let block_arg = (
            (location(), Identifier::parser(())).skip(spaced(token(':'))),
            type_parser().skip(spaces()),
        );

        // Parse the entire block header as a single combine chain so that failures
        // after consuming the label+args are reported as committed errors.
        // Format: ^label(args) [optional_attrs] [!N]:
        let outline_loc = state_stream.loc();
        let (label, args, attrs_opt, outindex_opt) = (
            spaced(token('^').with(Identifier::parser(()))),
            spaced(delimited_list_parser('(', ')', ',', block_arg)),
            spaced(optional(AttributeDict::parser(()))),
            spaces().with(optional(token('!').with(usize::parser(())))),
        )
            .skip(spaced(token(':')))
            .parse_stream(state_stream)
            .into_result()?
            .0;

        // Parse the ops in the block
        let ops: Vec<_> = spaces()
            .with(sep_by::<Vec<_>, _, _, _>(
                Operation::parser(OperationParserConfig {
                    look_for_outlined_attrs: false,
                })
                .skip(spaces()),
                token(';').skip(spaces()),
            ))
            .parse_stream(state_stream)
            .into_result()?
            .0;

        // We've parsed the components. Now construct the result.
        let (arg_names, arg_types): (Vec<_>, Vec<_>) = args.into_iter().unzip();
        let block = BasicBlock::new(state_stream.state.ctx, Some(label.clone()), arg_types);

        // Set the location of the block to be from where we just parsed it.
        // If it has a different one specified as part of its outlined information,
        // that will be overwritten later.
        block.deref_mut(state_stream.state.ctx).set_loc(loc.clone());

        // Set seen attributes.
        if let Some(attrs) = attrs_opt {
            block.deref_mut(state_stream.state.ctx).attributes = attrs;
        }

        for (arg_idx, (arg_loc, name)) in arg_names.into_iter().enumerate() {
            let def: Value = block.deref(state_stream.state.ctx).args[arg_idx].as_value(block);
            state_stream.state.name_tracker.ssa_def(
                state_stream.state.ctx,
                &(name.clone(), arg_loc),
                def,
            )?;
            set_block_arg_name(state_stream.state.ctx, block, arg_idx, Some(name));
        }

        // Register in outline parse state if !N was found.
        if let Some(outindex) = outindex_opt {
            register_block_for_outline(state_stream, outindex, block, outline_loc)?;
        }

        for op in ops {
            op.insert_at_back(block, state_stream.state.ctx);
        }
        state_stream
            .state
            .name_tracker
            .block_def(state_stream.state.ctx, &(label, loc), block)?;
        Ok(block).into_parse_result()
    }
}
