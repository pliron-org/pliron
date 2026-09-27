// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Builtin dialect op interfaces

use crate::{
    basic_block::BasicBlock,
    builtin::{
        attributes::{OperandSegmentSizesAttr, TypeAttr},
        type_interfaces::FunctionTypeInterface,
    },
    context::{Context, Ptr},
    dict_key,
    graph::walkers::interruptible::{WalkResult, walk_advance, walk_break},
    identifier::Identifier,
    linked_list::ContainsLinkedList,
    location::{Located, Location},
    op::{Op, op_cast},
    operation::Operation,
    printable::Printable,
    region::Region,
    result::{AnyError, Result},
    symbol_table::{SymbolTableCollection, walk_symbol_table},
    r#type::{Type, TypeHandle, TypeInterfaceMarker, Typed, type_impls},
    utils::{const_bound_n::LessThanN, table::HMap},
    value::Value,
    verify_err, verify_error,
};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::ops::Range;
use pliron::derive::op_interface;
use thiserror::Error;

use super::attributes::IdentifierAttr;

/// An [Op] implementing this interface is a block terminator.
#[op_interface]
pub trait IsTerminatorInterface {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum BranchOpInterfaceVerifyErr {
    #[error("Branch Op is passing {provided} arguments, but target block expects {expected}")]
    SuccessorOperandsMismatch { provided: usize, expected: usize },
    #[error("Forwarded operand at {idx} is of type {forwarded}, but should've been {expected}")]
    SuccessorOperandTypeMismatch {
        idx: usize,
        forwarded: String,
        expected: String,
    },
}

/// This [terminator](IsTerminatorInterface) [Op] branches to
/// other [BasicBlock]s, possibly passing arguments to the target block.
///
/// This is similar to MLIR's [BranchOpInterface], but stricter:
///
/// 1. Produced operands aren't supported, just forwarded.
/// 2. Type of the value passed is expected to be the same as the target block argument.
///
/// [BranchOpInterface]: https://github.com/llvm/llvm-project/blob/b1f04d57f5818914d7db506985e2932f217844bd/mlir/include/mlir/Interfaces/ControlFlowInterfaces.td
#[op_interface]
pub trait BranchOpInterface: IsTerminatorInterface {
    /// Verify that
    ///  - Calling [successor_operand_range](Self::successor_operand_range)
    ///    for any `succ_idx < Operation::get_num_successors()` does not panic.
    ///  - The operand range it returns is contained in `0..Operation::get_num_operands()`.
    ///
    /// Typically, an impl will include a call to `<Self as OperandSegmentInterface>::verify`.
    fn verify_successor_operand_layout(&self, ctx: &Context) -> Result<()>;

    /// Return the index range of operands forwarded to successor `succ_idx`.
    /// The `i`th returned index identifies the operand for the target block's `i`th argument.
    /// Panics if `succ_idx` is invalid.
    fn successor_operand_range(&self, ctx: &Context, succ_idx: usize) -> Range<usize>;

    /// Get the list of [Value]s forwarded to successor `succ_idx`.
    /// Panics if `succ_idx` is invalid.
    fn successor_operands(&self, ctx: &Context, succ_idx: usize) -> Vec<Value> {
        let range = self.successor_operand_range(ctx, succ_idx);
        let op = self.get_operation().deref(ctx);
        range.map(|opd_idx| op.get_operand(opd_idx)).collect()
    }

    /// Replace the operand forwarded to argument `arg_idx` of successor `succ_idx` with `operand`.
    /// Panics if `succ_idx` or `arg_idx` is invalid.
    fn set_successor_operand(
        &self,
        ctx: &Context,
        succ_idx: usize,
        arg_idx: usize,
        operand: Value,
    ) {
        let range = self.successor_operand_range(ctx, succ_idx);
        assert!(
            arg_idx < range.len(),
            "Successor argument index {arg_idx} out of bounds for {} operands forwarded to successor {succ_idx}",
            range.len()
        );
        Operation::replace_operand(self.get_operation(), ctx, range.start + arg_idx, operand);
    }

    /// Add a new operand to be forwarded to the given successor.
    /// The operand is appended after existing operands for the specified successor.
    /// Returns the index of the newly added operand among the operands forwarded to the successor.
    /// The returned index can be used to determine the corresponding target block argument index.
    /// Panics if `succ_idx` is invalid.
    fn add_successor_operand(&self, ctx: &mut Context, succ_idx: usize, operand: Value) -> usize;

    /// Remove and return the operand forwarded to argument `arg_idx` of successor `succ_idx`.
    /// Panics if `succ_idx` or `arg_idx` is invalid.
    fn remove_successor_operand(&self, ctx: &mut Context, succ_idx: usize, arg_idx: usize)
    -> Value;

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op_cast::<dyn BranchOpInterface>(op).unwrap();
        // Verify that we can call [Self::successor_operands] and use its results without a panic.
        self_op.verify_successor_operand_layout(ctx)?;

        // Verify that the values passed to a target block
        // matches the arguments of that block.
        for (succ_idx, succ) in op.get_operation().deref(ctx).successors().enumerate() {
            let succ = &*succ.deref(ctx);
            let operands = self_op.successor_operands(ctx, succ_idx);
            if succ.get_num_arguments() != operands.len() {
                return verify_err!(
                    op.loc(ctx),
                    BranchOpInterfaceVerifyErr::SuccessorOperandsMismatch {
                        provided: operands.len(),
                        expected: succ.get_num_arguments()
                    }
                );
            }
            for (idx, operand) in operands.iter().enumerate() {
                let block_arg = succ.get_argument(idx);
                if operand.get_type(ctx) != block_arg.get_type(ctx) {
                    return verify_err!(
                        op.loc(ctx),
                        BranchOpInterfaceVerifyErr::SuccessorOperandTypeMismatch {
                            idx,
                            forwarded: operand.get_type(ctx).disp(ctx).to_string(),
                            expected: block_arg.get_type(ctx).disp(ctx).to_string(),
                        }
                    );
                }
            }
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Expected {0} successor(s), but found {1}")]
pub struct NSuccsVerifyErr(pub usize, pub usize);

/// An [Op] having exactly `N` successors. Successors are branch targets, so this
/// requires [BranchOpInterface] (which separately checks the forwarded operands
/// against each target block's arguments).
#[op_interface]
pub trait NSuccsInterface<const N: usize>: BranchOpInterface {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let opr = op.get_operation();
        let op = &*opr.deref(ctx);
        if op.get_num_successors() != N {
            return verify_err!(op.loc(), NSuccsVerifyErr(N, op.get_num_successors()));
        }
        Ok(())
    }

    /// Get the `i`'th successor block.
    fn get_successor_i(&self, ctx: &Context, i: LessThanN<N>) -> Ptr<BasicBlock> {
        self.get_operation().deref(ctx).get_successor(i.i())
    }
}

/// An [Op] having exactly one successor.
#[op_interface]
pub trait OneSuccInterface: BranchOpInterface {
    /// Get the single successor block of this [Op].
    fn get_successor(&self, ctx: &Context) -> Ptr<BasicBlock> {
        self.get_operation().deref(ctx).get_successor(0)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);
        if op.get_num_successors() != 1 {
            return verify_err!(op.loc(), NSuccsVerifyErr(1, op.get_num_successors()));
        }
        Ok(())
    }
}

dict_key!(
    /// Key for the `operand_segment_sizes` attribute.
    ATTR_KEY_OPERAND_SEGMENT_SIZES, "builtin_operand_segment_sizes"
);

#[derive(Error, Debug)]
/// Error returned when verifying an [OperandSegmentInterface] operation
pub enum OperandSegmentInterfaceVerifyErr {
    #[error("operand_segment_sizes attribute not found")]
    OperandSegmentSizesAttrErr,
    #[error("operand_segment_sizes total {0} does not match the number of operands {1}")]
    OperandSegmentSizesTotalMismatchErr(u32, u32),
    #[error("Expected {expected} operand segments, but found {found}")]
    SegmentCountMismatch { expected: usize, found: usize },
}

/// Interface for operations whose operands are grouped into segments.
///
/// In the case of variadic operands, sometimes it makes sense to group
/// contiguous operands together into a segment. This interface aids doing that.
/// MLIR achieves this by having ODS (tablegen)' `AttrSizedOperandSegments` generate
/// `getODSOperands()` based on the `operandSegmentSizes` attribute.
///
/// ### Attribute(s):
/// | Name | Static Name Identifier | Type |
/// |------|------------------------| -----|
/// | builtin_operand_segment_sizes | [ATTR_KEY_OPERAND_SEGMENT_SIZES] | [OperandSegmentSizesAttr](crate::builtin::attributes::OperandSegmentSizesAttr) |
#[op_interface]
pub trait OperandSegmentInterface {
    /// The number of operand segments that this [Op] must have,
    /// or [None] if any number of segments is valid.
    fn expected_num_segments(&self, ctx: &Context) -> Option<usize>;

    /// Given a list of segmented operands, compute the segment sizes and flatten the operands
    /// (ready for use in constructing an operation).
    /// Call `set_operand_segment_sizes` with the computed segment sizes to set the attribute.
    fn compute_segment_sizes(operands: Vec<Vec<Value>>) -> (Vec<Value>, OperandSegmentSizesAttr)
    where
        Self: Sized,
    {
        let sizes = operands
            .iter()
            .map(|seg| seg.len().try_into().unwrap())
            .collect::<Vec<_>>();
        let flat_operands = operands.into_iter().flatten().collect();

        let sizes_attr = OperandSegmentSizesAttr(sizes);
        (flat_operands, sizes_attr)
    }

    /// Return the index range of operands in segment `seg_idx` of this [Op].
    /// Panics if `seg_idx` is out of bounds.
    fn segment_range(&self, ctx: &Context, seg_idx: usize) -> Range<usize> {
        let sizes = self.get_operand_segment_sizes(ctx).0;
        assert!(
            seg_idx < sizes.len(),
            "Segment index {seg_idx} out of bounds for {} segments",
            sizes.len()
        );

        let start = sizes[..seg_idx].iter().sum::<u32>() as usize;
        start..start + sizes[seg_idx] as usize
    }

    /// Get the `seg_idx`th segment of operands.
    /// Panics if `seg_idx` is out of bounds.
    fn get_segment(&self, ctx: &Context, seg_idx: usize) -> Vec<Value> {
        let range = self.segment_range(ctx, seg_idx);
        let self_op = self.get_operation().deref(ctx);
        range.map(|opd_idx| self_op.get_operand(opd_idx)).collect()
    }

    /// Get the length of the `seg_idx`th segment.
    fn segment_size(&self, ctx: &Context, seg_idx: usize) -> u32 {
        let sizes = self.get_operand_segment_sizes(ctx).0;
        if seg_idx >= sizes.len() {
            return 0;
        }
        sizes[seg_idx]
    }

    /// Get the number of segments.
    fn num_segments(&self, ctx: &Context) -> usize {
        self.get_operand_segment_sizes(ctx).0.len()
    }

    /// Set the `operand_segment_sizes` attribute for this operation.
    fn set_operand_segment_sizes(&self, ctx: &Context, sizes: OperandSegmentSizesAttr) {
        let mut self_op = self.get_operation().deref_mut(ctx);
        self_op
            .attributes
            .set(ATTR_KEY_OPERAND_SEGMENT_SIZES.clone(), sizes);
    }

    /// Get the `operand_segment_sizes` attribute for this operation.
    fn get_operand_segment_sizes(&self, ctx: &Context) -> OperandSegmentSizesAttr {
        let self_op = self.get_operation().deref(ctx);
        self_op
            .attributes
            .get::<OperandSegmentSizesAttr>(&ATTR_KEY_OPERAND_SEGMENT_SIZES)
            .unwrap()
            .clone()
    }

    /// Push a new operand at the end of the `seg_idx`th segment.
    /// Returns the index of the inserted operand within that segment.
    fn push_to_segment(&self, ctx: &mut Context, seg_idx: usize, operand: Value) -> usize {
        let mut sizes = self.get_operand_segment_sizes(ctx).0;
        assert!(
            seg_idx < sizes.len(),
            "Segment index {seg_idx} out of bounds for {} segments",
            sizes.len()
        );

        let seg_opd_idx = sizes[seg_idx] as usize;
        let insert_idx = sizes[..=seg_idx].iter().sum::<u32>() as usize;
        Operation::insert_operand(self.get_operation(), ctx, insert_idx, operand);

        sizes[seg_idx] += 1;
        self.set_operand_segment_sizes(ctx, OperandSegmentSizesAttr(sizes));
        seg_opd_idx
    }

    /// Pop and return the last operand in the `seg_idx`th segment.
    fn pop_from_segment(&self, ctx: &mut Context, seg_idx: usize) -> Value {
        let mut sizes = self.get_operand_segment_sizes(ctx).0;
        assert!(
            seg_idx < sizes.len(),
            "Segment index {seg_idx} out of bounds for {} segments",
            sizes.len()
        );

        let segment_start = sizes[..seg_idx].iter().sum::<u32>() as usize;
        let segment_len = sizes[seg_idx] as usize;
        assert!(segment_len > 0, "Cannot pop from an empty segment");

        let remove_idx = segment_start + segment_len - 1;
        let removed = Operation::remove_operand(self.get_operation(), ctx, remove_idx);

        sizes[seg_idx] -= 1;
        self.set_operand_segment_sizes(ctx, OperandSegmentSizesAttr(sizes));
        removed
    }

    /// Insert an operand at `seg_opd_idx` within the `seg_idx`th segment.
    fn insert_into_segment(
        &self,
        ctx: &mut Context,
        seg_idx: usize,
        seg_opd_idx: usize,
        operand: Value,
    ) {
        let mut sizes = self.get_operand_segment_sizes(ctx).0;
        assert!(
            seg_idx < sizes.len(),
            "Segment index {seg_idx} out of bounds for {} segments",
            sizes.len()
        );

        let segment_start = sizes[..seg_idx].iter().sum::<u32>() as usize;
        let segment_len = sizes[seg_idx] as usize;
        assert!(
            seg_opd_idx <= segment_len,
            "Segment operand index {seg_opd_idx} out of bounds for insertion in segment of length {segment_len}"
        );

        let insert_idx = segment_start + seg_opd_idx;
        Operation::insert_operand(self.get_operation(), ctx, insert_idx, operand);

        sizes[seg_idx] += 1;
        self.set_operand_segment_sizes(ctx, OperandSegmentSizesAttr(sizes));
    }

    /// Remove and return the operand at `seg_opd_idx` in the `seg_idx`th segment.
    fn remove_from_segment(&self, ctx: &mut Context, seg_idx: usize, seg_opd_idx: usize) -> Value {
        let mut sizes = self.get_operand_segment_sizes(ctx).0;
        assert!(
            seg_idx < sizes.len(),
            "Segment index {seg_idx} out of bounds for {} segments",
            sizes.len()
        );

        let segment_start = sizes[..seg_idx].iter().sum::<u32>() as usize;
        let segment_len = sizes[seg_idx] as usize;
        assert!(
            seg_opd_idx < segment_len,
            "Segment operand index {seg_opd_idx} out of bounds for removal in segment of length {segment_len}"
        );

        let remove_idx = segment_start + seg_opd_idx;
        let removed = Operation::remove_operand(self.get_operation(), ctx, remove_idx);

        sizes[seg_idx] -= 1;
        self.set_operand_segment_sizes(ctx, OperandSegmentSizesAttr(sizes));
        removed
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        let Some(attr) = self_op
            .attributes
            .get::<OperandSegmentSizesAttr>(&ATTR_KEY_OPERAND_SEGMENT_SIZES)
        else {
            return verify_err!(
                self_op.loc(),
                OperandSegmentInterfaceVerifyErr::OperandSegmentSizesAttrErr
            );
        };

        let total = attr.0.iter().cloned().sum::<u32>();

        let num_operands: u32 = self_op.get_num_operands().try_into().unwrap();
        if total != num_operands {
            return verify_err!(
                self_op.loc(),
                OperandSegmentInterfaceVerifyErr::OperandSegmentSizesTotalMismatchErr(
                    total,
                    num_operands
                )
            );
        }

        let segmented_op = op_cast::<dyn OperandSegmentInterface>(op).unwrap();
        let found = attr.0.len();
        if let Some(expected) = segmented_op.expected_num_segments(ctx)
            && found != expected
        {
            return verify_err!(
                self_op.loc(),
                OperandSegmentInterfaceVerifyErr::SegmentCountMismatch { expected, found }
            );
        }

        Ok(())
    }
}

/// Describe the abstract semantics of [Regions](crate::region::Region).
///
/// See MLIR's [RegionKind](https://mlir.llvm.org/docs/Interfaces/#regionkindinterfaces).
pub enum RegionKind {
    /// Represents a graph region without control flow semantics.
    Graph,
    /// Represents an [SSA-style control](https://mlir.llvm.org/docs/LangRef/#control-flow-and-ssacfg-regions)
    /// flow region with basic blocks and reachability.
    SSACFG,
}

/// Info on contained [Regions](crate::region::Region).
#[op_interface]
pub trait RegionKindInterface {
    /// Return the kind of the region with the given index inside this operation.
    fn get_region_kind(&self, idx: usize) -> RegionKind;
    /// Return true if the region with the given index inside this operation
    /// must require dominance to hold.
    fn has_ssa_dominance(&self, idx: usize) -> bool {
        matches!(self.get_region_kind(idx), RegionKind::SSACFG)
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Expected {} regions, found {}", .0, .1)]
pub struct NRegionsVerifyErr(usize, usize);

/// [Op]s that have a fixed number of regions.
#[op_interface]
pub trait NRegionsInterface<const N: usize> {
    /// Checks that the operation has exactly one region.
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        if self_op.num_regions() != N {
            return verify_err!(self_op.loc(), NRegionsVerifyErr(N, self_op.num_regions()));
        }
        Ok(())
    }

    /// Get the `i`'th region.
    fn get_region_i(&self, ctx: &Context, i: LessThanN<N>) -> Ptr<Region> {
        self.get_operation().deref(ctx).get_region(i.i())
    }
}

/// [Op]s that have exactly one region.
#[op_interface]
pub trait OneRegionInterface {
    /// Get the single region that this [Op] has.
    fn get_region(&self, ctx: &Context) -> Ptr<Region> {
        self.get_operation().deref(ctx).get_region(0)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        if self_op.num_regions() != 1 {
            return verify_err!(self_op.loc(), NRegionsVerifyErr(1, self_op.num_regions()));
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("At most {0} regions expected, but found {1} regions")]
pub struct AtMostNRegionVerifyErr(usize, usize);

#[op_interface]
pub trait AtMostNRegionsInterface<const N: usize> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        let n_regions = self_op.num_regions();
        if n_regions > N {
            return verify_err!(self_op.loc(), AtMostNRegionVerifyErr(N, n_regions));
        }
        Ok(())
    }
}

/// [Op]s that have at most one region.
#[op_interface]
pub trait AtMostOneRegionInterface: AtMostNRegionsInterface<1> {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }

    fn get_region(&self, ctx: &Context) -> Option<Ptr<Region>> {
        let self_op = self.get_operation().deref(ctx);
        self_op.regions().next()
    }
}

#[derive(Error, Debug)]
#[error("Op {0} must only have regions with single block")]
pub struct SingleBlockRegionVerifyErr(String);

/// [Op]s with regions that have a single block.
#[op_interface]
pub trait SingleBlockRegionInterface {
    /// Get the single body block in `region_idx`.
    fn get_body(&self, ctx: &Context, region_idx: usize) -> Ptr<BasicBlock> {
        self.get_operation()
            .deref(ctx)
            .get_region(region_idx)
            .deref(ctx)
            .get_head()
            .expect("Expected SingleBlockRegion Op to contain a block")
    }

    /// Insert an operation at the end of the single block in `region_idx`.
    fn append_operation(&self, ctx: &mut Context, op: Ptr<Operation>, region_idx: usize) {
        op.insert_at_back(self.get_body(ctx, region_idx), ctx);
    }

    /// Checks that the operation has regions with single block.
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let opr = op.get_operation();
        let self_op = opr.deref(ctx);
        for region in self_op.regions() {
            if region.deref(ctx).iter(ctx).count() != 1 {
                return verify_err!(
                    self_op.loc(),
                    SingleBlockRegionVerifyErr(Operation::get_opid(opr, ctx).to_string())
                );
            }
        }
        Ok(())
    }
}

/// [Op]s whose single-basic-block regions need not have a terminator.
#[op_interface]
pub trait NoTerminatorInterface: SingleBlockRegionInterface {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

dict_key!(
    /// Key for symbol name attribute when the operation defines a symbol.
    ATTR_KEY_SYM_NAME, "builtin_sym_name"
);

#[derive(Error, Debug)]
#[error("Op implementing SymbolOpInterface does not have a symbol defined")]
pub struct SymbolOpInterfaceErr;

/// [Op] that defines or declares a [symbol](https://mlir.llvm.org/docs/SymbolsAndSymbolTables/#symbol).
///
/// ### Attribute(s):
/// | Name | Static Name Identifier | Type |
/// |------|------------------------| -----|
/// | builtin_sym_name | [ATTR_KEY_SYM_NAME] | [IdentifierAttr](crate::builtin::attributes::IdentifierAttr) |
#[op_interface]
pub trait SymbolOpInterface {
    /// Get the name of the symbol defined by this operation.
    fn get_symbol_name(&self, ctx: &Context) -> Identifier {
        let self_op = self.get_operation().deref(ctx);
        let s_attr = self_op
            .attributes
            .get::<IdentifierAttr>(&ATTR_KEY_SYM_NAME)
            .unwrap();
        s_attr.clone().into()
    }

    /// Set a name for the symbol defined by this operation.
    fn set_symbol_name(&self, ctx: &mut Context, name: Identifier) {
        let name_attr = IdentifierAttr::new(name);
        let mut self_op = self.get_operation().deref_mut(ctx);
        self_op.attributes.set(ATTR_KEY_SYM_NAME.clone(), name_attr);
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        if self_op
            .attributes
            .get::<IdentifierAttr>(&ATTR_KEY_SYM_NAME)
            .is_none()
        {
            return verify_err!(op.loc(ctx), SymbolOpInterfaceErr);
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum SymbolTableInterfaceErr {
    #[error("Multiple definitions of Symbol {0}")]
    SymbolRedefined(String),
}

// Any [Op] that holds a symbol table.
#[op_interface]
pub trait SymbolTableInterface: SingleBlockRegionInterface + OneRegionInterface {
    /// Lookup a symbol in this symbol table op. Linear search.
    fn lookup(&self, ctx: &Context, sym: &Identifier) -> Option<Ptr<Operation>> {
        for op in self.get_body(ctx, 0).deref(ctx).iter(ctx) {
            if let Some(sym_op) =
                op_cast::<dyn SymbolOpInterface>(Operation::get_op_dyn(op, ctx).as_ref())
                && &sym_op.get_symbol_name(ctx) == sym
            {
                return Some(op);
            }
        }
        None
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op_cast::<dyn SymbolTableInterface>(op).unwrap();

        // Check that every symbol is defined only once.
        let mut seen = HMap::<Identifier, Location>::default();
        let table_ops_block = op.get_body(ctx, 0);
        for op in table_ops_block.deref(ctx).iter(ctx) {
            if let Some(sym_op) =
                op_cast::<dyn SymbolOpInterface>(Operation::get_op_dyn(op, ctx).as_ref())
            {
                let sym = sym_op.get_symbol_name(ctx);
                if let Some(prev_loc) = seen.insert(sym.clone(), op.deref(ctx).loc()) {
                    return verify_err!(
                        op.deref(ctx).loc(),
                        verify_error!(
                            prev_loc,
                            SymbolTableInterfaceErr::SymbolRedefined(sym.to_string())
                        )
                    );
                }
            }
        }

        struct State {
            symbol_table_collection: SymbolTableCollection,
            res: Result<()>,
        }
        // Verify Ops inside that implement [SymbolUserOpInterface].
        fn callback(ctx: &Context, state: &mut State, op: Ptr<Operation>) -> WalkResult<()> {
            if let Some(sym_user_op) =
                op_cast::<dyn SymbolUserOpInterface>(Operation::get_op_dyn(op, ctx).as_ref())
                && let Err(err) =
                    sym_user_op.verify_symbol_uses(ctx, &mut state.symbol_table_collection)
            {
                state.res = Err(err);
                return walk_break(());
            }
            walk_advance()
        }

        let mut state = State {
            symbol_table_collection: SymbolTableCollection::new(),
            res: Ok(()),
        };
        walk_symbol_table(pliron::dyn_clone::clone_box(op), ctx, &mut state, callback);
        state.res
    }
}

#[op_interface]
pub trait SymbolUserOpInterface {
    /// Verify the symbol uses held by this operation. This is called when verifying
    /// a symbol table operation that (possibly transitively) contains this operation.
    fn verify_symbol_uses(
        &self,
        ctx: &Context,
        symbol_tables: &mut SymbolTableCollection,
    ) -> Result<()>;

    /// Returns the list of symbols used by this operation.
    fn used_symbols(&self, ctx: &Context) -> Vec<Identifier>;

    /// Nothing (by default) to verify for symbol users. Override if needed.
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Expected {0} result(s), but found {1} results")]
pub struct NResultsVerifyErr(pub usize, pub usize);

/// An [Op] having exactly N results.
#[op_interface]
pub trait NResultsInterface<const N: usize> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let opr = op.get_operation();
        let op = &*opr.deref(ctx);
        if op.get_num_results() != N {
            return verify_err!(op.loc(), NResultsVerifyErr(N, op.get_num_results()));
        }
        Ok(())
    }

    /// Get the `i`'th result.
    fn get_result_i(&self, ctx: &Context, i: LessThanN<N>) -> Value {
        self.get_operation().deref(ctx).get_result(i.i())
    }

    /// Get the type of the `i`'th result.
    fn result_type_i(&self, ctx: &Context, i: LessThanN<N>) -> TypeHandle {
        self.get_operation().deref(ctx).get_type(i.i())
    }
}

#[derive(Error, Debug)]
#[error("At most {0} results expected, but found {1} results")]
pub struct AtMostNResultsVerifyErr(usize, usize);

#[op_interface]
pub trait AtMostNResultsInterface<const N: usize> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        let n_results = self_op.get_num_results();
        if n_results > N {
            return verify_err!(self_op.loc(), AtMostNResultsVerifyErr(N, n_results));
        }
        Ok(())
    }
}

/// An [Op] having at most one result.
#[op_interface]
pub trait OptionalResultInterface {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        if self_op.get_num_results() > 1 {
            return verify_err!(
                self_op.loc(),
                AtMostNResultsVerifyErr(1, self_op.get_num_results())
            );
        }
        Ok(())
    }

    /// Get the single result defined by this [Op], if any.
    fn get_result_opt(&self, ctx: &Context) -> Option<Value> {
        let self_op = self.get_operation().deref(ctx);
        (self_op.get_num_results() == 1).then(|| self_op.get_result(0))
    }
}

#[derive(Error, Debug)]
#[error("Expected at least {0} results, but found {1} results")]
pub struct AtLeastNResultsVerifyErr(usize, usize);

/// An [Op] having at least N results.
#[op_interface]
pub trait AtLeastNResultsInterface<const N: usize> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        let n_results = self_op.get_num_results();
        if n_results < N {
            return verify_err!(self_op.loc(), AtLeastNResultsVerifyErr(N, n_results));
        }
        Ok(())
    }
}

/// An [Op] having exactly one result.
#[op_interface]
pub trait OneResultInterface {
    /// Get the single result defined by this [Op].
    fn get_result(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_result(0)
    }

    /// Get the type of the single result defined by this [Op].
    fn result_type(&self, ctx: &Context) -> TypeHandle {
        self.get_operation().deref(ctx).get_type(0)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        if self_op.get_num_results() != 1 {
            return verify_err!(
                self_op.loc(),
                NResultsVerifyErr(1, self_op.get_num_results())
            );
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Expected {} operand(s), but found {}", .0, .1)]
pub struct NOpdsVerifyErr(pub usize, pub usize);

/// An [Op] having exactly N operands.
#[op_interface]
pub trait NOpdsInterface<const N: usize> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let opr = op.get_operation();
        let op = &*opr.deref(ctx);
        if op.get_num_operands() != N {
            return verify_err!(op.loc(), NOpdsVerifyErr(N, op.get_num_operands()));
        }
        Ok(())
    }

    /// Get the `i`'th operand.
    fn get_operand_i(&self, ctx: &Context, i: LessThanN<N>) -> Value {
        self.get_operation().deref(ctx).get_operand(i.i())
    }

    /// Get the type of the `i`'th operand.
    fn operand_type_i(&self, ctx: &Context, i: LessThanN<N>) -> TypeHandle {
        self.get_operand_i(ctx, i).get_type(ctx)
    }
}

#[derive(Error, Debug)]
#[error("At most {0} operands expected, but found {1} operands")]
pub struct AtMostNOpdsVerifyErr(usize, usize);

#[op_interface]
pub trait AtMostNOpdsInterface<const N: usize> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        let n_operands = self_op.get_num_operands();
        if n_operands > N {
            return verify_err!(self_op.loc(), AtMostNOpdsVerifyErr(N, n_operands));
        }
        Ok(())
    }
}

/// An [Op] having at most one operand.
#[op_interface]
pub trait OptionalOpdInterface {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        if self_op.get_num_operands() > 1 {
            return verify_err!(
                self_op.loc(),
                AtMostNOpdsVerifyErr(1, self_op.get_num_operands())
            );
        }
        Ok(())
    }

    fn get_operand_opt(&self, ctx: &Context) -> Option<Value> {
        let self_op = self.get_operation().deref(ctx);
        (self_op.get_num_operands() == 1).then(|| self_op.get_operand(0))
    }
}

#[derive(Error, Debug)]
#[error("Expected at least {0} operands, but found {1} operands")]
pub struct AtLeastNOpdsVerifyErr(usize, usize);

/// An [Op] having at least N operands.
#[op_interface]
pub trait AtLeastNOpdsInterface<const N: usize> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        let n_operands = self_op.get_num_operands();
        if n_operands < N {
            return verify_err!(self_op.loc(), AtLeastNOpdsVerifyErr(N, n_operands));
        }
        Ok(())
    }
}

/// An [Op] having exactly one operand.
#[op_interface]
pub trait OneOpdInterface {
    /// Get the single operand used by this [Op].
    fn get_operand(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(0)
    }

    /// Get the type of the single operand used by this [Op].
    fn operand_type(&self, ctx: &Context) -> TypeHandle {
        self.get_operand(ctx).get_type(ctx)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op.get_operation().deref(ctx);
        if self_op.get_num_operands() != 1 {
            return verify_err!(self_op.loc(), NOpdsVerifyErr(1, self_op.get_num_operands()));
        }
        Ok(())
    }
}

/// An [Op] whose regions's SSA names are isolated from above.
/// This is similar to (but not the same as) MLIR's
/// [IsolatedFromAbove](https://mlir.llvm.org/docs/Traits/#isolatedfromabove) trait.
/// Definition: all regions that are reachable / traversible in any
/// direction in the region hierarchy without passing an `IsolatedFromAbove`
/// barrier, share the same SSA name space.
/// i.e., a region that is not `IsolatedFromAbove` cannot have any SSA name
/// in common with that of any of its ancestors or siblings or cousins etc.
#[op_interface]
pub trait IsolatedFromAboveInterface {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Op has different operand types")]
pub struct SameOperandsTypeVerifyErr;

/// An [Op]  with all operands having the same type.
#[op_interface]
pub trait SameOperandsType {
    /// Get the common type of the operands.
    fn common_operand_type(&self, ctx: &Context) -> Option<TypeHandle> {
        let self_op = self.get_operation().deref(ctx);
        (self_op.get_num_operands() > 0).then(|| self_op.get_operand(0).get_type(ctx))
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);

        let mut opds = op.operands();
        let Some(ty) = opds.next().map(|opd| opd.get_type(ctx)) else {
            return Ok(());
        };
        for opd in opds {
            if opd.get_type(ctx) != ty {
                return verify_err!(op.loc(), SameOperandsTypeVerifyErr);
            }
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Expected operand type {0}, but found {1}")]
pub struct AllOperandsOfTypeVerifyErr(String, String);

/// An [Op] with all operands having the specified type.
#[op_interface]
pub trait AllOperandsOfType<T: Type> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);

        for opd in op.operands() {
            let opd_ty = &*opd.get_type(ctx).deref(ctx);
            if !opd_ty.as_any().is::<T>() {
                return verify_err!(
                    op.loc(),
                    AllOperandsOfTypeVerifyErr(
                        T::get_type_id_static().disp(ctx).to_string(),
                        opd_ty.disp(ctx).to_string()
                    )
                );
            }
        }

        Ok(())
    }
}

/// Error from an [Op] interface that checks operand types
/// against a [type interface](TypeInterfaceMarker).
#[derive(Error, Debug)]
pub enum TypeInterfaceImplsErr {
    #[error("Expected operand {0} to implement {1}, but {2} does not")]
    Operand(usize, String, String),
    #[error("Expected result {0} to implement {1}, but {2} does not")]
    Result(usize, String, String),
    #[error("Expected operand segment {0} to implement {1}, but {2} does not")]
    Segment(usize, String, String),
}

/// Name of the [type interface](TypeInterfaceMarker) `I`, for error messages.
fn interface_name<I: ?Sized + TypeInterfaceMarker + 'static>() -> String {
    core::any::type_name::<I>().to_string()
}

/// An [Op] with all operands implementing the specified [type interface](TypeInterfaceMarker).
#[op_interface]
pub trait AllOperandsImplsTy<I: ?Sized + TypeInterfaceMarker + 'static> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);

        for (idx, opd) in op.operands().enumerate() {
            let opd_ty = &*opd.get_type(ctx).deref(ctx);
            if !type_impls::<I>(opd_ty) {
                return verify_err!(
                    op.loc(),
                    TypeInterfaceImplsErr::Operand(
                        idx,
                        interface_name::<I>(),
                        opd_ty.get_type_id().disp(ctx).to_string()
                    )
                );
            }
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Op has only {0} operands, but expected at least {1}")]
pub struct NotEnoughOperandsErr(pub usize, pub usize);

/// Verify that `op` has an operand at index `N` (0-indexed).
pub fn verify_get_operand_n<const N: usize>(op: Ptr<Operation>, ctx: &Context) -> Result<Value> {
    let opr = op.deref(ctx);
    if opr.get_num_operands() <= N {
        return verify_err!(opr.loc(), NotEnoughOperandsErr(opr.get_num_operands(), N));
    }
    Ok(opr.get_operand(N))
}

#[derive(Error, Debug)]
pub enum OperandNOfTypeError {
    #[error("Expected operand type {0}, but found {1}")]
    AllOperandsOfTypeVerifyErr(String, String),
}

/// An [Op] whose N-th operand (0-indexed) has the specified type.
#[op_interface]
pub trait OperandNOfType<const N: usize, T: Type> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let opd_n = verify_get_operand_n::<N>(op.get_operation(), ctx)?;
        let opd_n_ty = &*opd_n.get_type(ctx).deref(ctx);
        if !opd_n_ty.as_any().is::<T>() {
            return verify_err!(
                op.loc(ctx),
                OperandNOfTypeError::AllOperandsOfTypeVerifyErr(
                    T::get_type_id_static().disp(ctx).to_string(),
                    opd_n_ty.get_type_id().disp(ctx).to_string()
                )
            );
        }

        Ok(())
    }
}

/// An [Op] whose N-th operand (0-indexed) implements the specified
/// [type interface](TypeInterfaceMarker).
#[op_interface]
pub trait OperandNImplsTy<const N: usize, I: ?Sized + TypeInterfaceMarker + 'static> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let opd_n = verify_get_operand_n::<N>(op.get_operation(), ctx)?;
        let opd_n_ty = &*opd_n.get_type(ctx).deref(ctx);
        if !type_impls::<I>(opd_n_ty) {
            return verify_err!(
                op.loc(ctx),
                TypeInterfaceImplsErr::Operand(
                    N,
                    interface_name::<I>(),
                    opd_n_ty.get_type_id().disp(ctx).to_string()
                )
            );
        }

        Ok(())
    }
}

/// Outcome of [resolve_index_range].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IndexRange {
    /// Indices `start` to `end`, both inclusive.
    Range { start: usize, end: usize },
    /// No index is in the range.
    Empty,
    /// The list must have at least this many entities.
    TooFewEntities(usize),
}

/// Resolve the inclusive index range `[m, n]` over a list of `len` entities.
///   - A negative `n` counts backwards: `-1` is the last entity.
///   - [IndexRange::Empty]: `n` is negative and the list is too short.
///   - [IndexRange::TooFewEntities]: `n` is not negative and the list has no index `n`.
fn resolve_index_range(m: u32, n: i32, len: usize) -> IndexRange {
    let end = if n >= 0 {
        let end = n as usize;
        if len <= end {
            return IndexRange::TooFewEntities(end + 1);
        }
        end
    } else {
        let from_end = n.unsigned_abs() as usize;
        if from_end > len {
            return IndexRange::Empty;
        }
        len - from_end
    };

    let start = m as usize;
    if start > end {
        return IndexRange::Empty;
    }
    IndexRange::Range { start, end }
}

/// Verify the type of every entity of `entities` that is in the inclusive index `range`,
/// which is `(m, n)` as in [resolve_index_range].
///
///   - `check`: Check if the type matches a requirement.
///   - `too_few`: Build an error for when the list is too short.
///     Gets the length of the list, and the length that is required.
///   - `mismatch`: Build an error for the first entity that fails `check`.
///     Gets the index of that entity, and its type.
fn verify_index_range_types<E1: AnyError, E2: AnyError>(
    ctx: &Context,
    loc: Location,
    entities: impl Iterator<Item = Value> + Clone,
    range: (u32, i32),
    check: impl Fn(&dyn Type) -> bool,
    too_few: impl FnOnce(usize, usize) -> E1,
    mismatch: impl FnOnce(usize, &dyn Type) -> E2,
) -> Result<()> {
    let (m, n) = range;
    let num_entities = entities.clone().count();
    let (start, end) = match resolve_index_range(m, n, num_entities) {
        IndexRange::Range { start, end } => (start, end),
        IndexRange::Empty => return Ok(()),
        IndexRange::TooFewEntities(required) => {
            return verify_err!(loc, too_few(num_entities, required));
        }
    };

    for (idx, entity) in entities.enumerate().skip(start).take(end - start + 1) {
        let ty = &*entity.get_type(ctx).deref(ctx);
        if !check(ty) {
            return verify_err!(loc, mismatch(idx, ty));
        }
    }

    Ok(())
}

#[derive(Error, Debug)]
pub enum OperandsMNOfTypeError {
    #[error("Op has only {0} operands, but expected at least {1}")]
    NotEnoughOperands(usize, usize),
    #[error("Expected operand {0} to be of type {1}, but found {2}")]
    UnexpectedType(usize, String, String),
}

/// An [Op] whose operands in the inclusive index range `[M, N]` (0-indexed) are of type `T`.
///
///   - `N < 0`: `N` counts backwards from the end. `-1` is the last operand. For example,
///     `OperandsMNOfType<1, -1, T>` specifies that every operand after the first is a `T`.
///   - `N >= 0`: The [Op] must have an operand at index `N`.
#[op_interface]
pub trait OperandsMNOfType<const M: u32, const N: i32, T: Type> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        const {
            assert!(
                N < 0 || N as u32 >= M,
                "OperandsMNOfType: M must not be greater than N"
            );
        }

        let self_op = op.get_operation().deref(ctx);
        verify_index_range_types(
            ctx,
            self_op.loc(),
            self_op.operands(),
            (M, N),
            |ty| ty.as_any().is::<T>(),
            OperandsMNOfTypeError::NotEnoughOperands,
            |idx, ty| {
                OperandsMNOfTypeError::UnexpectedType(
                    idx,
                    T::get_type_id_static().disp(ctx).to_string(),
                    ty.get_type_id().disp(ctx).to_string(),
                )
            },
        )
    }
}

/// An [Op] whose operands in the inclusive index range `[M, N]` (0-indexed) implement
/// the specified [type interface](TypeInterfaceMarker).
///
///   - `N < 0`: `N` counts backwards from the end. `-1` is the last operand. For example,
///     `OperandsMNImplsTy<1, -1, I>` specifies that every operand after the first implements `I`.
///   - `N >= 0`: The [Op] must have an operand at index `N`.
#[op_interface]
pub trait OperandsMNImplsTy<const M: u32, const N: i32, I: ?Sized + TypeInterfaceMarker + 'static> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        const {
            assert!(
                N < 0 || N as u32 >= M,
                "OperandsMNImplsTy: M must not be greater than N"
            );
        }

        let self_op = op.get_operation().deref(ctx);
        verify_index_range_types(
            ctx,
            self_op.loc(),
            self_op.operands(),
            (M, N),
            type_impls::<I>,
            NotEnoughOperandsErr,
            |idx, ty| {
                TypeInterfaceImplsErr::Operand(
                    idx,
                    interface_name::<I>(),
                    ty.get_type_id().disp(ctx).to_string(),
                )
            },
        )
    }
}

#[derive(Error, Debug)]
pub enum SegmentNOfTypeError {
    #[error("Op does not have operand segment at index {0}")]
    SegmentNotFound(usize),
    #[error("Expected operand segment type {0}, but found {1}")]
    UnexpectedType(String, String),
}

/// An [Op] whose N-th operand segment (0-indexed) has the specified type.
#[op_interface]
pub trait SegmentNOfType<const N: usize, T: Type>: OperandSegmentInterface {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let segmented_op = op_cast::<dyn OperandSegmentInterface>(op)
            .expect("Op must impl OperandSegmentInterface");
        if N >= segmented_op.num_segments(ctx) {
            return verify_err!(op.loc(ctx), SegmentNOfTypeError::SegmentNotFound(N));
        }

        for operand in segmented_op.get_segment(ctx, N) {
            let operand_ty = &*operand.get_type(ctx).deref(ctx);
            if !operand_ty.as_any().is::<T>() {
                return verify_err!(
                    op.loc(ctx),
                    SegmentNOfTypeError::UnexpectedType(
                        T::get_type_id_static().disp(ctx).to_string(),
                        operand_ty.disp(ctx).to_string()
                    )
                );
            }
        }

        Ok(())
    }
}

/// An [Op] whose N-th operand segment (0-indexed) implements the specified
/// [type interface](TypeInterfaceMarker).
#[op_interface]
pub trait SegmentNImplsTy<const N: usize, I: ?Sized + TypeInterfaceMarker + 'static>:
    OperandSegmentInterface
{
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let segmented_op = op_cast::<dyn OperandSegmentInterface>(op)
            .expect("Op must impl OperandSegmentInterface");
        if N >= segmented_op.num_segments(ctx) {
            return verify_err!(op.loc(ctx), SegmentNOfTypeError::SegmentNotFound(N));
        }

        for operand in segmented_op.get_segment(ctx, N) {
            let operand_ty = &*operand.get_type(ctx).deref(ctx);
            if !type_impls::<I>(operand_ty) {
                return verify_err!(
                    op.loc(ctx),
                    TypeInterfaceImplsErr::Segment(
                        N,
                        interface_name::<I>(),
                        operand_ty.get_type_id().disp(ctx).to_string()
                    )
                );
            }
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Op has different result types")]
pub struct SameResultsTypeVerifyErr;

// An [Op] with all results having the same type.
#[op_interface]
pub trait SameResultsType {
    /// Get the common type of the results.
    fn common_result_type(&self, ctx: &Context) -> Option<TypeHandle> {
        let self_op = self.get_operation().deref(ctx);
        (self_op.get_num_results() > 0).then(|| self_op.get_result(0).get_type(ctx))
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);

        let mut results = op.results();
        let Some(ty) = results.next().map(|result| result.get_type(ctx)) else {
            return Ok(());
        };
        for res in results {
            if res.get_type(ctx) != ty {
                return verify_err!(op.loc(), SameResultsTypeVerifyErr);
            }
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Expected result type {0}, but found {1}")]
pub struct AllResultsOfTypeVerifyErr(String, String);

/// An [Op] with all results having the specified type.
#[op_interface]
pub trait AllResultsOfType<T: Type> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);
        for res in op.results() {
            let res_ty = &*res.get_type(ctx).deref(ctx);
            if !res_ty.as_any().is::<T>() {
                return verify_err!(
                    op.loc(),
                    AllResultsOfTypeVerifyErr(
                        T::get_type_id_static().disp(ctx).to_string(),
                        res_ty.disp(ctx).to_string()
                    )
                );
            }
        }
        Ok(())
    }
}

/// An [Op] with all results implementing the specified [type interface](TypeInterfaceMarker).
#[op_interface]
pub trait AllResultsImplsTy<I: ?Sized + TypeInterfaceMarker + 'static> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);
        for (idx, res) in op.results().enumerate() {
            let res_ty = &*res.get_type(ctx).deref(ctx);
            if !type_impls::<I>(res_ty) {
                return verify_err!(
                    op.loc(),
                    TypeInterfaceImplsErr::Result(
                        idx,
                        interface_name::<I>(),
                        res_ty.get_type_id().disp(ctx).to_string()
                    )
                );
            }
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Op has only {0} results, but expected at least {1}")]
pub struct NotEnoughResultsErr(pub usize, pub usize);

/// Verify that `op` has a result at index `N` (0-indexed).
pub fn verify_get_result_n<const N: usize>(op: Ptr<Operation>, ctx: &Context) -> Result<Value> {
    let opr = op.deref(ctx);
    if opr.get_num_results() <= N {
        return verify_err!(opr.loc(), NotEnoughResultsErr(opr.get_num_results(), N));
    }
    Ok(opr.get_result(N))
}

#[derive(Error, Debug)]
pub enum ResultNOfTypeError {
    #[error("Expected result type {0}, but found {1}")]
    AllResultsOfTypeVerifyErr(String, String),
}

/// An [Op] whose N-th result (0-indexed) has the specified type.
#[op_interface]
pub trait ResultNOfType<const N: usize, T: Type> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let res_n = verify_get_result_n::<N>(op.get_operation(), ctx)?;
        let res_n_ty = &*res_n.get_type(ctx).deref(ctx);
        if !res_n_ty.as_any().is::<T>() {
            return verify_err!(
                op.loc(ctx),
                ResultNOfTypeError::AllResultsOfTypeVerifyErr(
                    T::get_type_id_static().disp(ctx).to_string(),
                    res_n_ty.get_type_id().disp(ctx).to_string()
                )
            );
        }
        Ok(())
    }
}

/// An [Op] whose N-th result (0-indexed) implements the specified
/// [type interface](TypeInterfaceMarker).
#[op_interface]
pub trait ResultNImplsTy<const N: usize, I: ?Sized + TypeInterfaceMarker + 'static> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let res_n = verify_get_result_n::<N>(op.get_operation(), ctx)?;
        let res_n_ty = &*res_n.get_type(ctx).deref(ctx);
        if !type_impls::<I>(res_n_ty) {
            return verify_err!(
                op.loc(ctx),
                TypeInterfaceImplsErr::Result(
                    N,
                    interface_name::<I>(),
                    res_n_ty.get_type_id().disp(ctx).to_string()
                )
            );
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum ResultsMNOfTypeError {
    #[error("Op has only {0} results, but expected at least {1}")]
    NotEnoughResults(usize, usize),
    #[error("Expected result {0} to be of type {1}, but found {2}")]
    UnexpectedType(usize, String, String),
}

/// An [Op] whose results in the inclusive index range `[M, N]` (0-indexed) are of type `T`.
///
///   - `N < 0`: `N` counts backwards from the end. `-1` is the last result. For example,
///     `ResultsMNOfType<1, -1, T>` specifies that every result after the first is a `T`.
///   - `N >= 0`: The [Op] must have a result at index `N`.
#[op_interface]
pub trait ResultsMNOfType<const M: u32, const N: i32, T: Type> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        const {
            assert!(
                N < 0 || N as u32 >= M,
                "ResultsMNOfType: M must not be greater than N"
            );
        }

        let self_op = op.get_operation().deref(ctx);
        verify_index_range_types(
            ctx,
            self_op.loc(),
            self_op.results(),
            (M, N),
            |ty| ty.as_any().is::<T>(),
            ResultsMNOfTypeError::NotEnoughResults,
            |idx, ty| {
                ResultsMNOfTypeError::UnexpectedType(
                    idx,
                    T::get_type_id_static().disp(ctx).to_string(),
                    ty.get_type_id().disp(ctx).to_string(),
                )
            },
        )
    }
}

/// An [Op] whose results in the inclusive index range `[M, N]` (0-indexed) implement
/// the specified [type interface](TypeInterfaceMarker).
///
///   - `N < 0`: `N` counts backwards from the end. `-1` is the last result. For example,
///     `ResultsMNImplsTy<1, -1, I>` specifies that every result after the first implements `I`.
///   - `N >= 0`: The [Op] must have a result at index `N`.
#[op_interface]
pub trait ResultsMNImplsTy<const M: u32, const N: i32, I: ?Sized + TypeInterfaceMarker + 'static> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        const {
            assert!(
                N < 0 || N as u32 >= M,
                "ResultsMNImplsTy: M must not be greater than N"
            );
        }

        let self_op = op.get_operation().deref(ctx);
        verify_index_range_types(
            ctx,
            self_op.loc(),
            self_op.results(),
            (M, N),
            type_impls::<I>,
            NotEnoughResultsErr,
            |idx, ty| {
                TypeInterfaceImplsErr::Result(
                    idx,
                    interface_name::<I>(),
                    ty.get_type_id().disp(ctx).to_string(),
                )
            },
        )
    }
}

#[derive(Error, Debug)]
#[error("Op has different operand and result types")]
pub struct SameOperandsAndResultTypeVerifyErr;

/// An [Op] with all results and operands having the same type.
#[op_interface]
pub trait SameOperandsAndResultType: SameOperandsType + SameResultsType {
    /// Get the common type of results / operands.
    fn common_type(&self, ctx: &Context) -> Option<TypeHandle> {
        self.common_result_type(ctx)
            .or_else(|| self.common_operand_type(ctx))
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let res_ty = op_cast::<dyn SameResultsType>(op)
            .expect("Op must impl SameResultsType")
            .common_result_type(ctx);
        let opd_ty = op_cast::<dyn SameOperandsType>(op)
            .expect("Op must impl SameOperandsType")
            .common_operand_type(ctx);

        if let (Some(res_ty), Some(opd_ty)) = (res_ty, opd_ty)
            && res_ty != opd_ty
        {
            return verify_err!(op.loc(ctx), SameOperandsAndResultTypeVerifyErr);
        }

        Ok(())
    }
}

/// A callable object is either a
///   - direct callee, expressed as a symbol)
///   - indirect callee, a [Value] pointing to the function to be called.
#[derive(Clone)]
pub enum CallOpCallable {
    Direct(Identifier),
    Indirect(Value),
}

#[derive(Error, Debug)]
pub enum CallOpInterfaceErr {
    #[error("Callee type attribute not found")]
    CalleeTypeAttrNotFoundErr,
    #[error("Callee type attribute must impl FunctionTypeInterface")]
    CalleeTypeAttrIncorrectTypeErr,
}

dict_key!(ATTR_KEY_CALLEE_TYPE, "builtin_callee_type");

/// A call-like op: Transfers control from one function to another.
/// See MLIR's [CallOpInterface](https://mlir.llvm.org/docs/Interfaces/#callinterfaces).
///
/// ### Attribute(s):
///
/// | Name | Static Name Identifier | Type |
/// |------|------------------------| -----|
/// | builtin_callee_type | [ATTR_KEY_CALLEE_TYPE] | [TypeAttr](crate::builtin::attributes::TypeAttr) |
#[op_interface]
pub trait CallOpInterface {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);
        let Some(callee_type_attr) = op.attributes.get::<TypeAttr>(&ATTR_KEY_CALLEE_TYPE) else {
            return verify_err!(op.loc(), CallOpInterfaceErr::CalleeTypeAttrNotFoundErr);
        };
        if !type_impls::<dyn FunctionTypeInterface>(&*callee_type_attr.get_type(ctx).deref(ctx)) {
            return verify_err!(op.loc(), CallOpInterfaceErr::CalleeTypeAttrIncorrectTypeErr);
        }
        Ok(())
    }

    /// Get the function that this call op is calling
    ///   - A symbol if this is a direct call
    ///   - A value if this is an indirect call
    fn callee(&self, ctx: &Context) -> CallOpCallable;

    /// Get arguments passed to callee
    fn args(&self, ctx: &Context) -> Vec<Value>;

    /// Type of the callee
    fn callee_type(&self, ctx: &Context) -> TypeHandle {
        let self_op = self.get_operation().deref(ctx);
        self_op
            .attributes
            .get::<TypeAttr>(&ATTR_KEY_CALLEE_TYPE)
            .unwrap()
            .get_type(ctx)
    }

    /// Set callee type
    fn set_callee_type(&self, ctx: &mut Context, callee_ty: TypeHandle) {
        let mut self_op = self.get_operation().deref_mut(ctx);
        let ty_attr = TypeAttr::new(callee_ty);
        self_op
            .attributes
            .set(ATTR_KEY_CALLEE_TYPE.clone(), ty_attr);
    }
}

#[cfg(test)]
mod tests {
    use super::{IndexRange, resolve_index_range};

    #[test]
    fn test_resolve_index_range() {
        // A non negative `n` is an index.
        assert_eq!(
            resolve_index_range(1, 2, 4),
            IndexRange::Range { start: 1, end: 2 }
        );
        assert_eq!(
            resolve_index_range(0, 0, 1),
            IndexRange::Range { start: 0, end: 0 }
        );
        // The list must reach a non negative `n`.
        assert_eq!(resolve_index_range(0, 2, 2), IndexRange::TooFewEntities(3));
        assert_eq!(resolve_index_range(0, 0, 0), IndexRange::TooFewEntities(1));

        // A negative `n` counts backwards.
        assert_eq!(
            resolve_index_range(1, -1, 3),
            IndexRange::Range { start: 1, end: 2 }
        );
        assert_eq!(
            resolve_index_range(0, -2, 3),
            IndexRange::Range { start: 0, end: 1 }
        );
        assert_eq!(
            resolve_index_range(2, -1, 3),
            IndexRange::Range { start: 2, end: 2 }
        );

        // A short list with a negative `n` gives an empty range.
        assert_eq!(resolve_index_range(1, -1, 1), IndexRange::Empty);
        assert_eq!(resolve_index_range(0, -1, 0), IndexRange::Empty);
        assert_eq!(resolve_index_range(0, -2, 1), IndexRange::Empty);
    }
}
