//! An operation is the basic unit of execution in the IR.
//! The general idea is similar to MLIR's
//! [Operation](https://mlir.llvm.org/docs/LangRef/#operations)

use alloc::{format, string::ToString, vec, vec::Vec};
use core::marker::PhantomData;
use thiserror::Error;

use crate::{
    attribute::{AttributeDict, verify_attr},
    basic_block::{BasicBlock, BasicBlockVerifyErr},
    builtin::op_interfaces::{IsTerminatorInterface, SymbolOpInterface},
    combine::{Parser, attempt, parser::char::spaces, token},
    common_traits::{Named, RcShare, Verify},
    context::{Arena, Context, Ptr, private::ArenaObj},
    debug_info,
    graph::{
        self,
        dominance::DomInfo,
        walkers::{
            IRNode, WALKCONFIG_PREORDER_FORWARD,
            interruptible::{WalkResult, walk_advance, walk_break},
        },
    },
    identifier::Identifier,
    input_err,
    irfmt::{
        outlined::{self, parse_outlines, postparse_outline},
        parsers::{list_parser, location, spaced},
        printers::iter_with_sep,
    },
    linked_list::{LinkedList, private},
    location::{Located, Location},
    op::{ConcreteOpInfo, Op, OpId, OpObj, op_cast, op_impls},
    parsable::{self, Parsable, ParseResult, StateStream},
    printable::{self, Printable},
    region::Region,
    result::Result,
    r#type::{TypeObj, Typed},
    utils::vec_exns::VecExtns,
    value::{DefNode, DefTrait, DefUseParticipant, DefiningEntity, Use, UseNode, Value},
    verify_err, verify_error,
};

/// Represents the result of an [Operation].
pub(crate) struct OpResult {
    /// The def containing the list of this result's uses.
    pub(crate) def: DefNode<Value>,
    /// Unique ID of this value in [Context].
    pub(crate) val_uid: u64,
    /// [Type](crate::type::Type) of this operation result.
    pub(crate) ty: Ptr<TypeObj>,
}

impl OpResult {
    /// Create a new OpResult with the given type and a new unique value ID from the context.
    pub(crate) fn new(ctx: &Context, ty: Ptr<TypeObj>) -> OpResult {
        OpResult {
            def: DefNode::new(),
            val_uid: ctx.get_new_value_uid(),
            ty,
        }
    }

    /// Get the [Type](crate::type::Type) of this operation result.
    pub(crate) fn get_type(&self) -> Ptr<TypeObj> {
        self.ty
    }

    /// Set the [Type](crate::type::Type) of this operation result.
    pub(crate) fn set_type(&mut self, ty: Ptr<TypeObj>) {
        self.ty = ty;
    }

    /// Build a [Value] corresponding to this operation result.
    pub(crate) fn as_value(&self, op: Ptr<Operation>) -> Value {
        Value {
            defining_entity: DefiningEntity::Op(op),
            val_uid: self.val_uid,
        }
    }
}

impl Typed for OpResult {
    fn get_type(&self, _ctx: &Context) -> Ptr<TypeObj> {
        self.get_type()
    }
}

/// Links an [Operation] with other operations and the container [BasicBlock]
#[derive(Default)]
struct BlockLinks {
    /// Parent block of this operation.
    parent_block: Option<Ptr<BasicBlock>>,
    /// The next operation in the basic block's list of operations.
    next_op: Option<Ptr<Operation>>,
    /// The previous operation in the basic block's list of operations.
    prev_op: Option<Ptr<Operation>>,
}

impl BlockLinks {
    pub fn new() -> BlockLinks {
        BlockLinks::default()
    }
}

/// Basic unit of execution. May or may not be in a [BasicBlock].
pub struct Operation {
    /// A [Ptr] to self.
    self_ptr: Ptr<Operation>,
    /// For quick creation of an [OpObj] or concrete [Op] from [Self].
    concrete_op: ConcreteOpInfo,
    /// [Results](OpResult) defined by self.
    pub(crate) results: Vec<OpResult>,
    /// [Operand]s used by self.
    pub(crate) operands: Vec<Operand<Value>>,
    /// Control-flow-graph successors.
    pub(crate) successors: Vec<Operand<Ptr<BasicBlock>>>,
    /// Links to the parent [BasicBlock] and
    /// previous and next [Operation]s in the block.
    block_links: BlockLinks,
    /// A dictionary of attributes.
    pub attributes: AttributeDict,
    /// Regions contained inside this operation.
    pub(crate) regions: Vec<Ptr<Region>>,
    /// Source location of this operation.
    loc: Location,
}

impl PartialEq for Operation {
    fn eq(&self, other: &Self) -> bool {
        self.self_ptr == other.self_ptr
    }
}

impl private::LinkedList for Operation {
    type ContainerType = BasicBlock;
    fn set_next(&mut self, next: Option<Ptr<Self>>) {
        self.block_links.next_op = next;
    }
    fn set_prev(&mut self, prev: Option<Ptr<Self>>) {
        self.block_links.prev_op = prev;
    }
    fn set_container(&mut self, container: Option<Ptr<BasicBlock>>) {
        self.block_links.parent_block = container;
    }
}

impl LinkedList for Operation {
    fn get_next(&self) -> Option<Ptr<Self>> {
        self.block_links.next_op
    }
    fn get_prev(&self) -> Option<Ptr<Self>> {
        self.block_links.prev_op
    }
    fn get_container(&self) -> Option<Ptr<BasicBlock>> {
        self.block_links.parent_block
    }
}

impl Operation {
    /// Create a new, unlinked (i.e., not in a basic block) operation.
    pub fn new(
        ctx: &mut Context,
        concrete_op: ConcreteOpInfo,
        result_types: Vec<Ptr<TypeObj>>,
        operands: Vec<Value>,
        successors: Vec<Ptr<BasicBlock>>,
        num_regions: usize,
    ) -> Ptr<Operation> {
        let f = |self_ptr: Ptr<Operation>| Operation {
            self_ptr,
            concrete_op,
            results: Vec::with_capacity(result_types.len()),
            operands: Vec::with_capacity(operands.len()),
            successors: Vec::with_capacity(successors.len()),
            block_links: BlockLinks::new(),
            attributes: AttributeDict::default(),
            regions: Vec::with_capacity(num_regions),
            loc: Location::Unknown,
        };

        // Create the new Operation.
        let newop = Self::alloc(ctx, f);
        // Update the results (we can't do this easily during creation).
        let results = result_types
            .into_iter()
            .map(|ty| OpResult::new(ctx, ty))
            .collect();
        newop.deref_mut(ctx).results = results;
        // Update the operands (we can't do this easily during creation).
        let operands = operands
            .iter()
            .map(|def| Operand::new(ctx, *def, newop))
            .collect();
        newop.deref_mut(ctx).operands = operands;
        let successors = successors
            .iter()
            .map(|def| Operand::new(ctx, *def, newop))
            .collect();
        newop.deref_mut(ctx).successors = successors;
        newop.deref_mut(ctx).regions = Vec::new_init(num_regions, |_| Region::new(ctx, newop));

        newop
    }

    /// Get parent block.
    pub fn get_parent_block(&self) -> Option<Ptr<BasicBlock>> {
        self.block_links.parent_block
    }

    /// Get parent region.
    pub fn get_parent_region(&self, ctx: &Context) -> Option<Ptr<Region>> {
        self.get_parent_block()
            .and_then(|block| block.deref(ctx).get_parent_region())
    }

    /// Get parent operation.
    pub fn get_parent_op(&self, ctx: &Context) -> Option<Ptr<Operation>> {
        self.get_parent_block()
            .and_then(|block| block.deref(ctx).get_parent_op(ctx))
    }

    /// Number of results this operation has.
    pub fn get_num_results(&self) -> usize {
        self.results.len()
    }

    /// Get idx'th result as a Value. Panics on invalid index.
    pub fn get_result(&self, idx: usize) -> Value {
        self.results[idx].as_value(self.self_ptr)
    }

    /// Get an iterator over the results of this operation.
    pub fn results(&self) -> impl Iterator<Item = Value> + Clone + '_ {
        self.results.iter().map(|res| res.as_value(self.self_ptr))
    }

    /// Add a result to the end of the result list, returning its index.
    pub fn push_result(this: Ptr<Self>, ctx: &Context, ty: Ptr<TypeObj>) -> usize {
        let new_result = OpResult::new(ctx, ty);
        this.deref_mut(ctx).results.push_back(new_result)
    }

    /// Remove the last result. Panics if there are no results or if the result has uses.
    /// Any [Value] referring to the removed result is invalidated.
    pub fn pop_result(this: Ptr<Self>, ctx: &Context) {
        let len = this.deref(ctx).results.len();
        assert!(
            len > 0,
            "Can't pop result from operation {} with no results",
            OpDbg { op: this, ctx }
        );
        Self::remove_result(this, ctx, len - 1);
    }

    /// Insert a new result at `res_idx`, shifting existing results, from `res_idx`, to the right.
    /// Panics on invalid index.
    pub fn insert_result(this: Ptr<Self>, ctx: &Context, res_idx: usize, ty: Ptr<TypeObj>) {
        let new_res = OpResult::new(ctx, ty);
        this.deref_mut(ctx).results.insert(res_idx, new_res);
        debug_info::insert_operation_result_name(ctx, this, res_idx, None);
    }

    /// Remove the result at `res_idx`, shifting existing results, from `res_idx + 1`, to the left.
    /// Panics on invalid index or if the removed result has uses.
    /// Any [Value] referring to the removed result is invalidated.
    pub fn remove_result(this: Ptr<Self>, ctx: &Context, res_idx: usize) {
        let value = this.deref(ctx).get_result(res_idx);
        assert!(
            !value.is_used(ctx),
            "Can't remove result {} from op {} with uses",
            res_idx,
            OpDbg { op: this, ctx }
        );
        debug_info::remove_operation_result_name(ctx, this, res_idx);
        this.deref_mut(ctx).results.remove(res_idx);
    }

    /// Does any result of this operation have a use?
    pub fn has_use(&self) -> bool {
        self.results.iter().any(|res| res.def.is_used())
    }

    /// Total number of uses (across all results).
    pub fn num_uses(&self) -> usize {
        self.results
            .iter()
            .fold(0, |count, res| count + res.def.num_uses())
    }

    ///  Get all uses of all results of this operation.
    pub fn uses(&self) -> impl Iterator<Item = Use<Value>> + '_ {
        self.results.iter().flat_map(|res| res.def.uses())
    }

    /// Get type of the idx'th result. Panics on invalid index.
    pub fn get_type(&self, idx: usize) -> Ptr<TypeObj> {
        self.results[idx].ty
    }

    /// Get an iterator over the result types of this operation.
    pub fn result_types(&self) -> impl Iterator<Item = Ptr<TypeObj>> + Clone + '_ {
        self.results.iter().map(|res| res.ty)
    }

    /// Get number of operands.
    pub fn get_num_operands(&self) -> usize {
        self.operands.len()
    }

    /// Get opd_idx'th operand of this [Operation]. Panics on invalid index.
    pub fn get_operand(&self, opd_idx: usize) -> Value {
        self.operands[opd_idx].get_def()
    }

    /// Get opd_idx'th operand as a [`Use<Value>`]. Panics on invalid index.
    pub fn get_operand_as_use(&self, opd_idx: usize) -> Use<Value> {
        self.operands[opd_idx].as_use(self.self_ptr)
    }

    /// Get an iterator over the operands of this operation.
    pub fn operands(&self) -> impl Iterator<Item = Value> + Clone + '_ {
        self.operands.iter().map(Operand::get_def)
    }

    /// Get an iterator over the operands of this operation as [`Use<Value>`]s.
    pub fn operands_as_uses(&self) -> impl Iterator<Item = Use<Value>> + '_ {
        self.operands
            .iter()
            .map(move |opd| opd.as_use(self.self_ptr))
    }

    /// Add a new operand to the end of the operand list, returning its index.
    pub fn push_operand(this: Ptr<Operation>, ctx: &Context, new_opd: Value) -> usize {
        let new_operand = Operand::new(ctx, new_opd, this);
        this.deref_mut(ctx).operands.push_back(new_operand)
    }

    /// Remove the last operand. Panics if there are no operands.
    /// Any [`Use<Value>`](Use) of the removed operand is invalidated.
    /// The removed [Value] is returned for convenience.
    pub fn pop_operand(this: Ptr<Operation>, ctx: &Context) -> Value {
        let len = this.deref(ctx).operands.len();
        assert!(
            len > 0,
            "Can't pop operand from operation {} with no operands",
            OpDbg { op: this, ctx }
        );
        Self::remove_operand(this, ctx, len - 1)
    }

    /// Replace opd_idx'th operand of `this` with `other`. Panics on invalid index.
    /// Any [`Use<Value>`](Use) of the replaced operand will be invalidated.
    pub fn replace_operand(this: Ptr<Operation>, ctx: &Context, opd_idx: usize, other: Value) {
        let new_operand = Operand::new(ctx, other, this);
        core::mem::replace(&mut this.deref_mut(ctx).operands[opd_idx], new_operand)
            .drop_use(ctx, this);
    }

    /// Insert a new operand at `opd_idx`, shifting existing operands, from `opd_idx`,
    /// to the right. Panics on invalid index (i.e., `opd_idx` > number of operands).
    pub fn insert_operand(this: Ptr<Operation>, ctx: &Context, opd_idx: usize, new_opd: Value) {
        let new_opd = Operand::new(ctx, new_opd, this);
        this.deref_mut(ctx).operands.insert(opd_idx, new_opd);
    }

    /// Remove the operand at `opd_idx`, shifting existing operands, from `opd_idx + 1`,
    /// to the left. Panics on invalid index (i.e., `opd_idx` >= number of operands).
    /// Any [`Use<Value>`](Use) of the removed operand is invalidated.
    /// The removed [Value] is returned for convenience.
    pub fn remove_operand(this: Ptr<Operation>, ctx: &Context, opd_idx: usize) -> Value {
        let removed_opd = this.deref_mut(ctx).operands.remove(opd_idx);
        let removed_value = removed_opd.get_def();
        removed_opd.drop_use(ctx, this);
        removed_value
    }

    /// Get number of successors
    pub fn get_num_successors(&self) -> usize {
        self.successors.len()
    }

    /// Get the opd_idx'th successor of this [Operation]. Panics on invalid index.
    pub fn get_successor(&self, succ_idx: usize) -> Ptr<BasicBlock> {
        self.successors[succ_idx].get_def()
    }

    /// Get the opd_idx'th successor as a [`Use<Ptr<BasicBlock>>`]. Panics on invalid index.
    pub fn get_successor_as_use(&self, succ_idx: usize) -> Use<Ptr<BasicBlock>> {
        self.successors[succ_idx].as_use(self.self_ptr)
    }

    /// Replace opd_idx'th successor of `this` with `other`. Panics on invalid index.
    /// Any [`Use<Ptr<BasicBlock>>`](Use) of the replaced successor will be invalidated.
    pub fn replace_successor(
        this: Ptr<Operation>,
        ctx: &Context,
        succ_idx: usize,
        other: Ptr<BasicBlock>,
    ) {
        let new_successor = Operand::new(ctx, other, this);
        core::mem::replace(&mut this.deref_mut(ctx).successors[succ_idx], new_successor)
            .drop_use(ctx, this);
    }

    /// Add a new successor to the end of the successor list, returning its index.
    pub fn push_successor(this: Ptr<Operation>, ctx: &Context, new_succ: Ptr<BasicBlock>) -> usize {
        let new_successor = Operand::new(ctx, new_succ, this);
        this.deref_mut(ctx).successors.push_back(new_successor)
    }

    /// Remove the last successor. Panics if there are no successors.
    /// Any [`Use<Ptr<BasicBlock>>`](Use) of the removed successor is invalidated.
    /// The removed `Ptr<BasicBlock>` is returned for convenience.
    pub fn pop_successor(this: Ptr<Operation>, ctx: &Context) -> Ptr<BasicBlock> {
        let len = this.deref(ctx).successors.len();
        assert!(
            len > 0,
            "Can't pop successor from operation {} with no successors",
            OpDbg { op: this, ctx }
        );
        Self::remove_successor(this, ctx, len - 1)
    }

    /// Insert a new successor at `succ_idx`, shifting existing successors, from `succ_idx`,
    /// to the right. Panics on invalid index (i.e., `succ_idx` > number of successors).
    pub fn insert_successor(
        this: Ptr<Operation>,
        ctx: &Context,
        succ_idx: usize,
        new_succ: Ptr<BasicBlock>,
    ) {
        let new_succ = Operand::new(ctx, new_succ, this);
        this.deref_mut(ctx).successors.insert(succ_idx, new_succ);
    }

    /// Remove the successor at `succ_idx`, shifting existing successors, from `succ_idx + 1`,
    /// to the left. Panics on invalid index (i.e., `succ_idx` >= number of successors).
    /// Any [`Use<Ptr<BasicBlock>>`](Use) of the removed successor is invalidated.
    /// The removed `Ptr<BasicBlock>` is returned for convenience.
    pub fn remove_successor(
        this: Ptr<Operation>,
        ctx: &Context,
        succ_idx: usize,
    ) -> Ptr<BasicBlock> {
        let removed_succ = this.deref_mut(ctx).successors.remove(succ_idx);
        let removed_block = removed_succ.get_def();
        removed_succ.drop_use(ctx, this);
        removed_block
    }

    /// Get an iterator on the successors.
    pub fn successors(&self) -> impl Iterator<Item = Ptr<BasicBlock>> + Clone + '_ {
        self.successors.iter().map(|opd| opd.get_def())
    }

    /// Get an iterator over the successors of this operation as [`Use<Ptr<BasicBlock>>`]s.
    pub fn successors_as_uses(&self) -> impl Iterator<Item = Use<Ptr<BasicBlock>>> + '_ {
        self.successors
            .iter()
            .map(|succ| succ.as_use(self.self_ptr))
    }

    /// Create an [OpObj] corresponding to self.
    /// [get_op](Self::get_op) is more efficient if the concrete type is known.
    pub fn get_op_dyn(ptr: Ptr<Self>, ctx: &Context) -> OpObj {
        (ptr.deref(ctx).concrete_op.0)(ptr)
    }

    /// Creates the concrete [Op] corresponding to self.
    pub fn get_op<T: Op>(ptr: Ptr<Self>, ctx: &Context) -> Option<T> {
        Self::is_op::<T>(ptr, ctx).then_some(T::from_operation(ptr))
    }

    /// Is this operation an instance of the concrete [Op] `T`?
    pub fn is_op<T: Op>(ptr: Ptr<Self>, ctx: &Context) -> bool {
        ptr.deref(ctx).concrete_op.1 == T::get_concrete_op_info().1
    }

    /// Get the [OpId] this Operation. Builds an intermediate [OpObj].
    pub fn get_opid(ptr: Ptr<Self>, ctx: &Context) -> OpId {
        Self::get_op_dyn(ptr, ctx).get_opid()
    }

    /// Get a [Ptr] to the `reg_idx`th region. Panics on invalid index.
    pub fn get_region(&self, reg_idx: usize) -> Ptr<Region> {
        self.regions[reg_idx]
    }

    /// Number of regions.
    pub fn num_regions(&self) -> usize {
        self.regions.len()
    }

    /// Add a new empty region to the operation and return its [Ptr].
    pub fn add_region(ptr: Ptr<Self>, ctx: &mut Context) -> Ptr<Region> {
        let region = Region::new(ctx, ptr);
        ptr.deref_mut(ctx).regions.push(region);
        region
    }

    /// Erase `reg_idx`'th region. Affects the index of all regions after it.
    /// Panics on invalid index.
    pub fn erase_region(ptr: Ptr<Self>, ctx: &mut Context, reg_idx: usize) {
        let reg = ptr.deref(ctx).regions[reg_idx];
        Region::drop_all_uses(reg, ctx);
        ptr.deref_mut(ctx).regions.remove(reg_idx);
        ArenaObj::dealloc(reg, ctx);
    }

    /// Get an iterator on the regions.
    pub fn regions(&self) -> impl Iterator<Item = Ptr<Region>> + Clone + '_ {
        self.regions.iter().cloned()
    }

    /// Drop all uses that this operation holds.
    pub fn drop_all_uses(ptr: Ptr<Self>, ctx: &Context) {
        // The operands cease to be a use of their definitions.
        let operands = core::mem::take(&mut (ptr.deref_mut(ctx).operands));
        for opd in operands.into_iter() {
            opd.drop_use(ctx, ptr);
        }
        // The successors cease to be a use of their definitions.
        let successors = core::mem::take(&mut (ptr.deref_mut(ctx).successors));
        for succ in successors.into_iter() {
            succ.drop_use(ctx, ptr);
        }

        let regions = ptr.deref(ctx).regions.clone();
        for region in regions {
            Region::drop_all_uses(region, ctx);
        }
    }

    /// Unlink and deallocate this operation and everything that it contains.
    /// Panics if there are any uses of this operation.
    pub fn erase(ptr: Ptr<Self>, ctx: &mut Context) {
        Self::drop_all_uses(ptr, ctx);
        assert!(
            !ptr.deref(ctx).has_use(),
            "Operation {} with use in {} being erased",
            OpDbg { op: ptr, ctx },
            OpDbg {
                op: ptr.deref(ctx).uses().next().unwrap().user_op,
                ctx
            }
        );
        if ptr.is_linked(ctx) {
            ptr.unlink(ctx);
        }
        ArenaObj::dealloc(ptr, ctx);
    }

    /// Parse a top level operation from a [StateStream].
    /// Top level parser looks for outlined attributes.
    pub fn top_level_parse<'a>(
        state_stream: &mut parsable::StateStream<'a>,
    ) -> ParseResult<'a, Ptr<Self>> {
        Operation::parse(
            state_stream,
            OperationParserConfig {
                look_for_outlined_attrs: true,
            },
        )
    }

    /// A parser combinator to parse a top level operation.
    /// Top level parser looks for outlined attributes.
    pub fn top_level_parser<'a>()
    -> impl Parser<StateStream<'a>, Output = Ptr<Self>, PartialState = ()> + 'a {
        combine::parser(move |parsable_state: &mut StateStream<'a>| {
            Self::top_level_parse(parsable_state)
        })
    }
}

impl ArenaObj for Operation {
    fn get_arena(ctx: &Context) -> &Arena<Self> {
        &ctx.operations
    }
    fn get_arena_mut(ctx: &mut Context) -> &mut Arena<Self> {
        &mut ctx.operations
    }
    fn dealloc_sub_objects(ptr: Ptr<Self>, ctx: &mut Context) {
        let regions = ptr.deref(ctx).regions.clone();
        for region in regions {
            ArenaObj::dealloc(region, ctx);
        }
    }
    fn get_self_ptr(&self, _ctx: &Context) -> Ptr<Self> {
        self.self_ptr
    }
}

/// Container for a [Use] in an [Operation].
pub(crate) struct Operand<T: DefUseParticipant> {
    pub(crate) r#use: UseNode<T>,
    pub(crate) use_uid: u64,
}

impl<T: DefUseParticipant + DefTrait> Operand<T> {
    /// Get the definition of this use.
    fn get_def(&self) -> T {
        self.r#use.get_def()
    }

    /// Drop this use, removing self from its definition's uses list.
    fn drop_use(&self, ctx: &Context, op: Ptr<Operation>) {
        self.get_def().get_defnode_mut(ctx).remove_use(Use {
            user_op: op,
            use_uid: self.use_uid,
            _dummy: PhantomData,
        });
    }

    /// Create a new Operand in `user_op`.
    fn new(ctx: &Context, def: T, user_op: Ptr<Operation>) -> Operand<T> {
        let use_uid = ctx.get_new_use_uid();
        Operand {
            r#use: def.get_defnode_mut(ctx).add_use(
                def,
                Use {
                    user_op,
                    use_uid,
                    _dummy: PhantomData,
                },
            ),
            use_uid,
        }
    }

    /// Verify that self is a valid operand of `user_op`.
    fn verify(&self, ctx: &Context, user_op: Ptr<Operation>) -> Result<()> {
        let def = self.get_def();
        let r#use = Use {
            user_op,
            use_uid: self.use_uid,
            _dummy: PhantomData,
        };
        if !def.get_defnode_ref(ctx).has_use_of(&r#use) {
            let loc = user_op.deref(ctx).loc();
            verify_err!(loc, DefUseVerifyErr::OperandNotUseOfDef)
        } else {
            Ok(())
        }
    }

    /// Build a [Use] corresponding to this operand.
    fn as_use(&self, user_op: Ptr<Operation>) -> Use<T> {
        Use {
            user_op,
            use_uid: self.use_uid,
            _dummy: PhantomData,
        }
    }
}

impl<T: DefUseParticipant + Named> Printable for Operand<T> {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "{}", self.r#use.get_def().unique_name(ctx))
    }
}

impl<T: DefUseParticipant + Typed> Typed for Operand<T> {
    fn get_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        self.r#use.get_def().get_type(ctx)
    }
}

#[derive(Error, Debug)]

pub enum DefUseVerifyErr {
    #[error("Operand is not a use of its def")]
    OperandNotUseOfDef,
    #[error("Use of {0} not dominated by definition")]
    UseNotDominatedByDef(Identifier),
}

/// Verify that every value in the IR dominates all of its uses.
pub fn verify_value_dominance(ir: Ptr<Operation>, ctx: &Context) -> Result<()> {
    let dom_info = &mut DomInfo::default();

    fn check_value_use(
        ctx: &Context,
        dom_info: &mut DomInfo,
        value: Value,
    ) -> WalkResult<pliron::result::Error> {
        for r#use in value.uses(ctx) {
            let user_op = r#use.user_op();
            if !dom_info.value_strictly_dominates_op(ctx, value, user_op) {
                let loc = user_op.deref(ctx).loc();
                return walk_break(verify_error!(
                    loc,
                    DefUseVerifyErr::UseNotDominatedByDef(value.unique_name(ctx))
                ));
            }
        }
        walk_advance()
    }

    let walker = graph::walkers::interruptible::immutable::walk_op;
    fn walker_callback(
        ctx: &Context,
        dom_info: &mut DomInfo,
        ir_node: IRNode,
    ) -> WalkResult<pliron::result::Error> {
        match ir_node {
            IRNode::Operation(opr) => {
                for result in opr.deref(ctx).results() {
                    check_value_use(ctx, dom_info, result)?;
                }
                walk_advance()
            }
            IRNode::BasicBlock(block) => {
                for arg in block.deref(ctx).arguments() {
                    check_value_use(ctx, dom_info, arg)?;
                }
                walk_advance()
            }
            IRNode::Region(_region) => walk_advance(),
        }
    }
    match walker(
        ctx,
        dom_info,
        &WALKCONFIG_PREORDER_FORWARD,
        ir,
        walker_callback,
    ) {
        core::ops::ControlFlow::Continue(_) => {}
        core::ops::ControlFlow::Break(err) => return Err(err),
    }
    Ok(())
}

/// Verify an operation and all its nested regions and blocks.
pub fn verify_operation(opr: Ptr<Operation>, ctx: &Context) -> Result<()> {
    // Verify attributes, operand, successors, region, results, interfaces and the Op's verifier.
    opr.deref(ctx).verify(ctx)?;
    // Verify that every definition in the IR rooted at opr, dominates its uses.
    verify_value_dominance(opr, ctx)
}

impl Verify for Operation {
    fn verify(&self, ctx: &Context) -> Result<()> {
        fn verify_inner(opr: &Operation, ctx: &Context) -> Result<()> {
            opr.attributes
                .0
                .values()
                .try_for_each(|attr| verify_attr(&**attr, ctx))?;
            opr.operands
                .iter()
                .try_for_each(|opd| opd.verify(ctx, opr.self_ptr))?;
            opr.successors
                .iter()
                .try_for_each(|succ| succ.verify(ctx, opr.self_ptr))?;
            opr.regions
                .iter()
                .try_for_each(|region| region.verify(ctx))?;
            opr.results
                .iter()
                .try_for_each(|res| res.as_value(opr.self_ptr).verify(ctx))?;
            let op = &*Operation::get_op_dyn(opr.self_ptr, ctx);
            if op_impls::<dyn IsTerminatorInterface>(op) && opr.get_next().is_some() {
                let loc = opr.loc.clone();
                let parent_block = opr
                    .get_parent_block()
                    .expect("There's a next operation, so there must be a parent block");
                verify_err!(
                    loc,
                    BasicBlockVerifyErr::TerminatorNotLast(
                        parent_block.unique_name(ctx).disp(ctx).to_string()
                    )
                )?
            }
            op.verify_interfaces(ctx)?;
            op.verify(ctx)
        }
        verify_inner(self, ctx).inspect_err(|err| {
            log::error!(target: "verify_error","{} in operation:\n{}", err.disp(ctx),
                    OpDbg { op: self.self_ptr, ctx })
        })
    }
}

impl Printable for Operation {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        Self::get_op_dyn(self.self_ptr, ctx).fmt(ctx, state, f)?;
        outlined::preprint_outline_operation(ctx, self.self_ptr, state.share(), f)?;

        if self.get_parent_op(ctx).is_none() {
            outlined::print_outlines(ctx, state.share(), f)?;
        }

        Ok(())
    }
}

impl Located for Operation {
    fn loc(&self) -> Location {
        self.loc.clone()
    }

    fn set_loc(&mut self, loc: Location) {
        self.loc = loc;
    }
}

/// Configuration for the [Operation] parser.
#[derive(Clone)]
pub struct OperationParserConfig {
    /// If set, the parser will look for outlined attributes after parsing the operation.
    pub look_for_outlined_attrs: bool,
}

impl Parsable for Operation {
    type Arg = OperationParserConfig;
    type Parsed = Ptr<Operation>;

    // Look for either of
    // - res_1, res_2, ..., res_n = opid
    // - opid
    // and hand it over to the Op specific parser.
    fn parse<'a>(
        state_stream: &mut parsable::StateStream<'a>,
        arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let loc = state_stream.loc();
        let _src = loc
            .source()
            .expect("Location from Parsable must be Location::SrcPos");

        let results_opid = spaces()
            .with(combine::optional(attempt(
                list_parser(',', (location(), Identifier::parser(()))).skip(spaced(token('='))),
            )))
            .and(spaced(OpId::parser(())));

        results_opid
            .then(|(results_opt, opid)| {
                let loc = loc.clone();
                let results: Vec<_> = results_opt
                    .unwrap_or(vec![])
                    .into_iter()
                    .map(|(res_loc, id)| (id, (res_loc)))
                    .collect();
                combine::parser(move |parsable_state: &mut StateStream<'a>| {
                    let state = &parsable_state.state;
                    let dialect = state
                        .ctx
                        .dialects
                        .get(&opid.dialect)
                        .expect("Dialect name parsed but dialect isn't registered");
                    let Some(opid_parser) = dialect.ops.get(&opid) else {
                        input_err!(loc.clone(), "Unregistered Op {}", opid.disp(state.ctx))?
                    };
                    let op = opid_parser(&(), results.clone())
                        .parse_stream(parsable_state)
                        .map(|op| op.get_operation())
                        .into();

                    if let Ok((op, _)) = op {
                        // Set the location of the operation to be from where we just parsed it.
                        // If it has a different one specified as part of its outlined information,
                        // that will be overwritten with later.
                        op.deref_mut(parsable_state.state.ctx).set_loc(loc.clone());
                        // If there's an outline index, post-parse it.
                        postparse_outline(parsable_state, op)?;
                    }

                    if arg.look_for_outlined_attrs {
                        // If we are looking for outlined attributes, parse them now.
                        parse_outlines(parsable_state)?;
                    }

                    op
                })
            })
            .parse_stream(state_stream)
            .into()
    }
}

/// Print basic [Operation] information, mainly for logging / debugging.
///
/// Includes the operation's unique identifier, its operands, and its successors.
/// Does not include attributes or regions, and is not meant to be a user-facing print.
pub fn print_dbg(
    ctx: &Context,
    opr: Ptr<Operation>,
    f: &mut core::fmt::Formatter<'_>,
) -> core::fmt::Result {
    let sep = printable::ListSeparator::CharSpace(',');

    let op = Operation::get_op_dyn(opr, ctx);
    let opid = op.get_opid();
    let opr = opr.deref(ctx);
    let operands = iter_with_sep(opr.operands(), sep);

    let symbol_opt = match op_cast::<dyn SymbolOpInterface>(&*op) {
        Some(sym_op) => " @".to_string() + &sym_op.get_symbol_name(ctx).disp(ctx).to_string(),
        None => "".to_string(),
    };

    // Print [Ptr] representing this operation.
    write!(f, "[{:?}] ", opr.get_self_ptr(ctx))?;

    if opr.get_num_results() > 0 {
        let results = iter_with_sep(opr.results(), sep);
        write!(f, "{} = ", results.disp(ctx))?;
    }

    write!(
        f,
        "{}{} ({})",
        opid.disp(ctx),
        symbol_opt,
        operands.disp(ctx)
    )?;

    if opr.get_num_successors() > 0 {
        let successors = iter_with_sep(
            opr.successors()
                .map(|succ| format!("^{}", succ.unique_name(ctx))),
            sep,
        );
        write!(f, " [{}]", successors.disp(ctx))?;
    }

    if !op.loc(ctx).is_unknown() {
        write!(f, " [@{}]", op.loc(ctx).disp(ctx))?;
    }

    Ok(())
}

/// A helper type that implements [Display](core::fmt::Display) and [Debug](core::fmt::Debug)
/// for debug printing an [Operation] with [print_dbg].
pub struct OpDbg<'a> {
    pub op: Ptr<Operation>,
    pub ctx: &'a Context,
}

impl<'a> core::fmt::Display for OpDbg<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        print_dbg(self.ctx, self.op, f)
    }
}

impl<'a> core::fmt::Debug for OpDbg<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        print_dbg(self.ctx, self.op, f)
    }
}
