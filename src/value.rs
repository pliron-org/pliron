//! SSA [Value]s: Use-Def and Def-Use Graph.
//! At the core of the IR infrastructure are SSA def-use and use-def chains.
//! Use-def and def-use chains are composed of four key structures:
//!   - [Value] describes a value definition, either a block argument, or an operation result.
//!   - [`Ptr<BasicBlock>`] describes a block definition.
//!     Its uses are in the predecessor branch operations.
//!   - [Use] describes the use of a definition.
//!     This may describe either a [Value] use (as operand in an [Operation])
//!     or a [BasicBlock] use (as successor of an [Operation]).

use alloc::{format, vec::Vec};
use core::{
    cell::{Ref, RefMut},
    hash::Hash,
    marker::PhantomData,
};
use rustc_hash::FxHashSet;

use crate::{
    arg_error,
    basic_block::BasicBlock,
    common_traits::{Named, Verify},
    context::{Context, Ptr},
    debug_info,
    identifier::Identifier,
    linked_list::{ContainsLinkedList, LinkedList},
    location::{Located, Location},
    operation::{DefUseVerifyErr, Operation},
    printable::Printable,
    result::Result,
    r#type::{TypeObj, Typed},
    verify_err, verify_error,
};

/// def-use chains are implemented for [Value]s and `Ptr<BasicBlock>`.
pub(crate) trait DefUseParticipant: Copy + Hash + Eq {}
impl DefUseParticipant for Value {}
impl DefUseParticipant for Ptr<BasicBlock> {}

/// A def node contains a list of its uses.
pub(crate) struct DefNode<T: DefUseParticipant> {
    /// The list of uses of this Def.
    uses: FxHashSet<Use<T>>,
}

impl<T: DefUseParticipant> DefNode<T> {
    /// Create a new definition.
    pub(crate) fn new() -> DefNode<T> {
        DefNode {
            uses: FxHashSet::default(),
        }
    }

    /// Does the definition have a use?
    pub(crate) fn is_used(&self) -> bool {
        !self.uses.is_empty()
    }

    /// How many uses does this definition have?
    pub(crate) fn num_uses(&self) -> usize {
        self.uses.len()
    }

    /// Does this definition have a [Use] of `use` ?
    pub(crate) fn has_use_of(&self, r#use: &Use<T>) -> bool {
        self.uses.contains(r#use)
    }

    /// Get a reference to all [Use]es.
    pub(crate) fn uses(&self) -> impl Iterator<Item = Use<T>> + '_ {
        self.uses.iter().cloned()
    }

    /// This definition has a new use. Track it and return a corresponding [Use].
    /// The returned [UseNode] is to be used in constructing an operand etc.
    pub(crate) fn add_use(&mut self, self_descr: T, r#use: Use<T>) -> UseNode<T> {
        if !self.uses.insert(r#use) {
            panic!("Def: Attempt to insert an existing use");
        }
        UseNode { def: self_descr }
    }

    /// Remove `use` from the underlying definition.
    pub(crate) fn remove_use(&mut self, r#use: Use<T>) {
        if !self.uses.remove(&r#use) {
            panic!("Def: Attempt to remove a use that doesn't exist");
        }
    }

    /// Replace the given use of the underlying definition `this` with `other`.
    pub(crate) fn replace_use_with(ctx: &Context, this: &T, r#use: &Use<T>, other: &T)
    where
        T: DefTrait + UseTrait,
    {
        if core::ptr::eq(&*this.get_defnode_ref(ctx), &*other.get_defnode_ref(ctx)) {
            return;
        }

        // Add r#use as a use of `other` and replace the [UseNode].
        let new_use_node = other.get_defnode_mut(ctx).add_use(*other, *r#use);
        *T::get_usenode_mut(r#use, ctx) = new_use_node;

        // `this` will no longer have r#use as a use.
        this.get_defnode_mut(ctx).uses.remove(r#use);
    }
}

#[derive(Debug, thiserror::Error)]
enum UseError {
    #[error("Use does not correspond to any operand of its user operation")]
    OperandNotInUserOp,
    #[error("Use does not correspond to any successor of its user operation")]
    SuccessorNotInUserOp,
}

/// Interface for [UseNode] wrappers.
pub(crate) trait UseTrait: DefUseParticipant {
    /// Find the index of the operand/successor in the user operation that corresponds to this use.
    /// Returns [Err] if the use is not in its user operation's operands/successors.
    fn try_find_index(r#use: &Use<Self>, ctx: &Context) -> Result<usize>;
    /// Find the index of the operand/successor in the user operation that corresponds to this use.
    /// Panics if the use is not in its user operation's operands/successors.
    fn find(r#use: &Use<Self>, ctx: &Context) -> usize {
        Self::try_find_index(r#use, ctx)
            .expect("Use is not in its user operation's operands/successors")
    }
    /// Get a reference to the [UseNode] described by this use.
    fn get_usenode_ref<'a>(r#use: &Use<Self>, ctx: &'a Context) -> Ref<'a, UseNode<Self>>;
    /// Get a mutable reference to the [UseNode] described by this use.
    fn get_usenode_mut<'a>(r#use: &Use<Self>, ctx: &'a Context) -> RefMut<'a, UseNode<Self>>;
}

/// Interface for [DefNode] wrappers.
pub(crate) trait DefTrait: DefUseParticipant {
    /// Get a reference to the underlying [DefNode].
    fn get_defnode_ref<'a>(&self, ctx: &'a Context) -> Ref<'a, DefNode<Self>>;
    /// Get a mutable reference to the underlying [DefNode].
    fn get_defnode_mut<'a>(&self, ctx: &'a Context) -> RefMut<'a, DefNode<Self>>;
}

/// The defining entity of a [Value]:
/// Either an [Operation] (as its result) or a [BasicBlock] (as its argument).
/// Use [Value::find_index] to find the result / argument index in the defining entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefiningEntity {
    Op(Ptr<Operation>),
    Block(Ptr<BasicBlock>),
}

/// Describes a value definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value {
    pub(crate) val_uid: u64,
    pub(crate) defining_entity: DefiningEntity,
}

#[derive(Debug, thiserror::Error)]
pub enum ValueError {
    #[error("Invalid Value: Defining operation has no result corresponding to this value")]
    NotInOpResult,
    #[error("Invalid Value: Defining block has no argument corresponding to this value")]
    NotInBlockArgument,
}

impl Value {
    /// Get the defining entity
    pub fn defining_entity(&self) -> DefiningEntity {
        self.defining_entity
    }

    /// If the defining entity of this value is [Operation], return it. Otherwise, return [None].
    pub fn defining_op(&self) -> Option<Ptr<Operation>> {
        match self.defining_entity {
            DefiningEntity::Op(op) => Some(op),
            DefiningEntity::Block(_) => None,
        }
    }

    /// If the defining entity of this value is [BasicBlock], return it. Otherwise, return [None].
    pub fn defining_block(&self) -> Option<Ptr<BasicBlock>> {
        match self.defining_entity {
            DefiningEntity::Op(_) => None,
            DefiningEntity::Block(block) => Some(block),
        }
    }

    /// Find the (result / argument) index of this value in its defining entity.
    /// Returns [Err] if this value is not currently defined by its defining entity.
    pub fn try_find_index(&self, ctx: &Context) -> Result<usize> {
        match self.defining_entity {
            DefiningEntity::Op(op) => {
                let op = op.deref(ctx);
                op.results
                    .iter()
                    .position(|res| res.val_uid == self.val_uid)
                    .ok_or_else(|| arg_error!(op.loc(), ValueError::NotInOpResult))
            }
            DefiningEntity::Block(block) => {
                let block = block.deref(ctx);
                block
                    .args
                    .iter()
                    .position(|arg| arg.val_uid == self.val_uid)
                    .ok_or_else(|| arg_error!(block.loc(), ValueError::NotInBlockArgument))
            }
        }
    }

    /// Find the (result / argument) index of this value in its defining entity.
    /// Panics if this value is not currently defined by its defining entity.
    pub fn find_index(&self, ctx: &Context) -> usize {
        self.try_find_index(ctx)
            .expect("Value is not currently defined by its defining entity")
    }

    /// How many uses does this definition have?
    pub fn num_uses(&self, ctx: &Context) -> usize {
        self.get_defnode_ref(ctx).num_uses()
    }

    /// Get all uses of this value.
    pub fn uses(&self, ctx: &Context) -> Vec<Use<Value>> {
        self.get_defnode_ref(ctx).uses().collect()
    }

    /// Does this definition have any [Use]?
    pub fn is_used(&self, ctx: &Context) -> bool {
        self.get_defnode_ref(ctx).is_used()
    }

    /// Replace uses of the underlying definition, that satisfy `pred`, with `other`.
    pub fn replace_some_uses_with<P: Fn(&Context, &Use<Value>) -> bool>(
        &self,
        ctx: &Context,
        predicate: P,
        other: &Value,
    ) {
        // We collect because we don't want to keep the defnode locked up.
        let touched_uses: FxHashSet<_> = self
            .get_defnode_ref(ctx)
            .uses
            .iter()
            .filter(|r#use| predicate(ctx, r#use))
            .cloned()
            .collect();
        for r#use in &touched_uses {
            DefNode::replace_use_with(ctx, self, r#use, other);
        }
    }

    /// Replace all uses of the underlying definition with `other`.
    pub fn replace_all_uses_with(&self, ctx: &Context, other: &Value) {
        self.replace_some_uses_with(ctx, |_, _| true, other);
    }

    /// Replace the given use of `this` [Value] with `other`.
    pub fn replace_use_with(&self, ctx: &Context, r#use: Use<Value>, other: &Value) {
        DefNode::replace_use_with(ctx, self, &r#use, other);
    }

    /// Get this value's location
    pub fn loc(&self, ctx: &Context) -> Location {
        match self.defining_entity {
            DefiningEntity::Op(op) => op.deref(ctx).loc(),
            DefiningEntity::Block(block) => block.deref(ctx).loc(),
        }
    }

    /// Set this value's type.
    pub fn set_type(&self, ctx: &Context, ty: Ptr<TypeObj>) {
        let index = self.find_index(ctx);
        match self.defining_entity {
            DefiningEntity::Op(op) => op.deref_mut(ctx).results[index].set_type(ty),
            DefiningEntity::Block(block) => block.deref_mut(ctx).args[index].set_type(ctx, ty),
        }
    }

    /// Get the defining block of this value.
    pub fn get_defining_block(&self, ctx: &Context) -> Option<Ptr<BasicBlock>> {
        match self.defining_entity {
            DefiningEntity::Op(op) => op.deref(ctx).get_parent_block(),
            DefiningEntity::Block(block) => Some(block),
        }
    }
}

impl Verify for Value {
    // Check that the value's uses point back to it,
    fn verify(&self, ctx: &Context) -> Result<()> {
        for r#use in self.uses(ctx) {
            let opd_idx = <Self as UseTrait>::try_find_index(&r#use, ctx)
                .map_err(|_| verify_error!(self.loc(ctx), DefUseVerifyErr::OperandNotUseOfDef))?;
            let use_operand = r#use.user_op.deref(ctx).get_operand(opd_idx);
            if use_operand != *self {
                verify_err!(self.loc(ctx), DefUseVerifyErr::OperandNotUseOfDef)?;
            }
        }
        self.get_type(ctx).verify(ctx)
    }
}

impl Typed for Value {
    fn get_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        let index = self.find_index(ctx);
        match self.defining_entity {
            DefiningEntity::Op(op) => op.deref(ctx).results[index].get_type(),
            DefiningEntity::Block(block) => block.deref(ctx).args[index].get_type(ctx),
        }
    }
}

impl Named for Value {
    fn given_name(&self, ctx: &Context) -> Option<Identifier> {
        let index = self.find_index(ctx);
        match self.defining_entity {
            DefiningEntity::Op(op) => debug_info::get_operation_result_name(ctx, op, index),
            DefiningEntity::Block(block) => debug_info::get_block_arg_name(ctx, block, index),
        }
    }

    fn id(&self, _ctx: &Context) -> Identifier {
        Identifier::try_from(format!("v{}", self.val_uid)).unwrap()
    }
}

impl Printable for Value {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &crate::printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "{}", self.unique_name(ctx))
    }
}

impl DefTrait for Value {
    fn get_defnode_ref<'a>(&self, ctx: &'a Context) -> Ref<'a, DefNode<Self>> {
        let index = self.find_index(ctx);
        match self.defining_entity {
            DefiningEntity::Op(op) => {
                let op = op.deref(ctx);
                Ref::map(op, |opref| &opref.results[index].def)
            }
            DefiningEntity::Block(block) => {
                let block = block.deref(ctx);
                Ref::map(block, |blockref| &blockref.args[index].def)
            }
        }
    }

    fn get_defnode_mut<'a>(&self, ctx: &'a Context) -> RefMut<'a, DefNode<Self>> {
        let index = self.find_index(ctx);
        match self.defining_entity {
            DefiningEntity::Op(op) => {
                let op = op.deref_mut(ctx);
                RefMut::map(op, |opref| &mut opref.results[index].def)
            }
            DefiningEntity::Block(block) => {
                let block = block.deref_mut(ctx);
                RefMut::map(block, |blockref| &mut blockref.args[index].def)
            }
        }
    }
}

impl UseTrait for Value {
    fn try_find_index(r#use: &Use<Self>, ctx: &Context) -> Result<usize> {
        let op = r#use.user_op.deref(ctx);
        op.operands
            .iter()
            .position(|operand| operand.use_uid == r#use.use_uid)
            .ok_or_else(|| arg_error!(op.loc(), UseError::OperandNotInUserOp))
    }
    fn get_usenode_ref<'a>(r#use: &Use<Self>, ctx: &'a Context) -> Ref<'a, UseNode<Self>> {
        let index = <Self as UseTrait>::find(r#use, ctx);
        let op = r#use.user_op.deref(ctx);
        Ref::map(op, |opref| &opref.operands[index].r#use)
    }
    fn get_usenode_mut<'a>(r#use: &Use<Self>, ctx: &'a Context) -> RefMut<'a, UseNode<Value>> {
        let index = <Self as UseTrait>::find(r#use, ctx);
        let op = r#use.user_op.deref_mut(ctx);
        RefMut::map(op, |opref| &mut opref.operands[index].r#use)
    }
}

impl Ptr<BasicBlock> {
    /// Does this block a predecessor?
    pub fn has_pred(&self, ctx: &Context) -> bool {
        self.deref(ctx).preds.is_used()
    }

    /// Is `pred` a predecessor of this block?
    /// *Note*: O(n) in the number of predecessors of this block.
    pub fn is_pred(&self, ctx: &Context, pred: Ptr<BasicBlock>) -> bool {
        self.deref(ctx)
            .preds
            .uses()
            .any(|r#use| r#use.user_op.deref(ctx).get_parent_block() == Some(pred))
    }

    /// Number of predecessors to this block.
    pub fn num_preds(&self, ctx: &Context) -> usize {
        self.deref(ctx).preds.num_uses()
    }

    /// Get the `i`-th predecessor of this block. Panics if `i` is out of bounds.
    /// *Note*:
    ///     1. The order of predecessors is not guaranteed to be deterministic
    ///     across different platforms or rust compiler versions.
    ///     2. O(n) in the number of predecessors of this block.
    pub fn get_pred(&self, ctx: &Context, i: usize) -> Ptr<BasicBlock> {
        self.deref(ctx)
            .preds
            .uses()
            .nth(i)
            .expect("Predecessor index out of bounds")
            .user_op
            .deref(ctx)
            .get_container()
            .expect("Terminator branching to this block is not in any basic block")
    }

    /// Get all predecessors of this block.
    /// *Note*: The order of predecessors is not guaranteed to be deterministic
    /// across different platforms or rust compiler versions.
    pub fn preds(&self, ctx: &Context) -> Vec<Ptr<BasicBlock>> {
        self.deref(ctx)
            .preds
            .uses()
            .map(|r#use| {
                r#use
                    .user_op
                    .deref(ctx)
                    .get_container()
                    .expect("Terminator branching to this block is not in any basic block")
            })
            .collect()
    }

    /// Get uses (as successor operands in an Operation) of this block.
    pub fn uses(&self, ctx: &Context) -> Vec<Use<Ptr<BasicBlock>>> {
        self.deref(ctx).preds.uses().collect()
    }

    /// Checks whether self is a successor of `pred`.
    /// O(n) in the number of successors of `pred`.
    pub fn is_succ_of(&self, ctx: &Context, pred: Ptr<BasicBlock>) -> bool {
        // We do not check [Self::get_defnode_ref].uses here because
        // we'd have to go through them all. We do not have a Use<_>
        // object to directly check membership.
        pred.deref(ctx)
            .get_tail()
            .is_some_and(|pred_term| pred_term.deref(ctx).successors().any(|succ| self == &succ))
    }
    /// Retarget predecessors (that satisfy pred) to `other`.
    pub fn retarget_some_preds_to<P: Fn(&Context, Ptr<BasicBlock>) -> bool>(
        &self,
        ctx: &Context,
        predicate: P,
        other: Ptr<BasicBlock>,
    ) {
        let predicate = |ctx: &Context, r#use: &Use<Ptr<BasicBlock>>| {
            let pred_block = r#use
                .user_op
                .deref(ctx)
                .get_container()
                .expect("Predecessor block must be in a Region");
            predicate(ctx, pred_block)
        };

        // We collect because we don't want to keep the defnode locked up.
        let touched_uses: FxHashSet<_> = self
            .get_defnode_ref(ctx)
            .uses
            .iter()
            .filter(|r#use| predicate(ctx, r#use))
            .cloned()
            .collect();

        for r#use in &touched_uses {
            DefNode::replace_use_with(ctx, self, r#use, &other);
        }
    }

    /// Retarget all predecessors to `other`.
    pub fn retarget_all_preds_to(&self, ctx: &Context, other: Ptr<BasicBlock>) {
        self.retarget_some_preds_to(ctx, |_, _| true, other);
    }

    /// Retarget the given pred (`block_use`) of `this` [`Ptr<BasicBlock>`] to `other`.
    pub fn retarget_pred_to(
        &self,
        ctx: &Context,
        block_use: Use<Ptr<BasicBlock>>,
        other: Ptr<BasicBlock>,
    ) {
        DefNode::replace_use_with(ctx, self, &block_use, &other);
    }
}

impl Named for Ptr<BasicBlock> {
    fn given_name(&self, ctx: &Context) -> Option<Identifier> {
        self.deref(ctx).given_name(ctx)
    }
    fn id(&self, ctx: &Context) -> Identifier {
        self.deref(ctx).id(ctx)
    }
}

impl DefTrait for Ptr<BasicBlock> {
    fn get_defnode_ref<'a>(&self, ctx: &'a Context) -> Ref<'a, DefNode<Self>> {
        let block = self.deref(ctx);
        Ref::map(block, |blockref| &blockref.preds)
    }

    fn get_defnode_mut<'a>(&self, ctx: &'a Context) -> RefMut<'a, DefNode<Self>> {
        let block = self.deref_mut(ctx);
        RefMut::map(block, |blockref| &mut blockref.preds)
    }
}

impl UseTrait for Ptr<BasicBlock> {
    fn try_find_index(r#use: &Use<Self>, ctx: &Context) -> Result<usize> {
        let op = r#use.user_op.deref(ctx);
        op.successors
            .iter()
            .position(|succ| succ.use_uid == r#use.use_uid)
            .ok_or_else(|| arg_error!(op.loc(), UseError::SuccessorNotInUserOp))
    }
    fn get_usenode_ref<'a>(r#use: &Use<Self>, ctx: &'a Context) -> Ref<'a, UseNode<Self>> {
        let succ_idx = <Self as UseTrait>::find(r#use, ctx);
        let op = r#use.user_op.deref(ctx);
        Ref::map(op, |opref| &opref.successors[succ_idx].r#use)
    }
    fn get_usenode_mut<'a>(
        r#use: &Use<Ptr<BasicBlock>>,
        ctx: &'a Context,
    ) -> RefMut<'a, UseNode<Ptr<BasicBlock>>> {
        let succ_idx = <Self as UseTrait>::find(r#use, ctx);
        let op = r#use.user_op.deref_mut(ctx);
        RefMut::map(op, |opref| &mut opref.successors[succ_idx].r#use)
    }
}

/// A use node contains a pointer to its definition.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UseNode<T: DefUseParticipant> {
    /// The definition that this is a use of.
    def: T,
}

impl<T: DefUseParticipant> UseNode<T> {
    pub(crate) fn get_def(&self) -> T {
        self.def
    }
}

/// Describes a [Value] or [BasicBlock] use.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
#[allow(private_bounds)]
pub struct Use<T: DefUseParticipant> {
    /// Uses of a def can only be in an operation.
    pub(crate) user_op: Ptr<Operation>,
    /// The global (in the context) unique id of the use we're describing.
    pub(crate) use_uid: u64,
    /// Phantom data to keep track of whether this is a Value use or a BasicBlock use.
    pub(crate) _dummy: PhantomData<T>,
}

#[allow(private_bounds)]
impl<T: UseTrait> Use<T> {
    /// Find index of the operand/successor in the user operation that corresponds to this [Use].
    /// Returns [Err] if the [Use] is not in its user operation's operands/successors.
    pub fn try_find_index(&self, ctx: &Context) -> Result<usize> {
        T::try_find_index(self, ctx)
    }

    /// Find index of the operand/successor in the user operation that corresponds to this [Use].
    /// Panics if the [Use] is not in its user operation's operands/successors.
    pub fn find_index(&self, ctx: &Context) -> usize {
        T::find(self, ctx)
    }

    /// Get the definition that this is a use of.
    pub fn get_def(&self, ctx: &Context) -> T {
        UseTrait::get_usenode_ref(self, ctx).get_def()
    }

    /// Get the operation that is the user of this use.
    pub fn user_op(&self) -> Ptr<Operation> {
        self.user_op
    }
}

impl Typed for Use<Value> {
    fn get_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        self.get_def(ctx).get_type(ctx)
    }
}
