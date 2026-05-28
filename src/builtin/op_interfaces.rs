use pliron::derive::op_interface;
use rustc_hash::FxHashMap;
use thiserror::Error;

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
    result::Result,
    symbol_table::{SymbolTableCollection, walk_symbol_table},
    r#type::{Type, TypeObj, Typed, type_impls},
    value::Value,
    verify_err, verify_error,
};

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
/// This is similar to MLIR's
/// [BranchOpInterface](https://github.com/llvm/llvm-project/blob/b1f04d57f5818914d7db506985e2932f217844bd/mlir/include/mlir/Interfaces/ControlFlowInterfaces.td)
/// but is stricter: (1) Produced operands aren't supported, just forwarded.
/// (2) Type of the value passed is expected to be the same as the target block argument.
#[op_interface]
pub trait BranchOpInterface: IsTerminatorInterface {
    /// Get a list of [Value]s that are forwarded to the target block.
    fn successor_operands(&self, ctx: &Context, succ_idx: usize) -> Vec<Value>;

    /// Add a new operand to be forwarded to the given successor.
    /// The operand is appended after existing operands for the specified successor.
    /// Returns the index of the newly added operand among the operands forwarded to the successor.
    /// The returned index can be used to determine the corresponding target block argument index.
    /// Panics if `succ_idx` is invalid.
    fn add_successor_operand(&self, ctx: &mut Context, succ_idx: usize, operand: Value) -> usize;

    /// Remove and return the operand at `opd_idx` among the operands forwarded to successor `succ_idx`.
    /// Panics if `succ_idx` or `opd_idx` is invalid.
    fn remove_successor_operand(&self, ctx: &mut Context, succ_idx: usize, opd_idx: usize)
    -> Value;

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let self_op = op_cast::<dyn BranchOpInterface>(op).unwrap();
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

dict_key!(
    /// Key for the `operand_segment_sizes` attribute.
    ATTR_KEY_OPERAND_SEGMENT_SIZES, "operand_segment_sizes"
);

#[derive(Error, Debug)]
/// Error returned when verifying an [OperandSegmentInterface] operation
pub enum OperandSegmentInterfaceVerifyErr {
    #[error("operand_segment_sizes attribute not found")]
    OperandSegmentSizesAttrErr,
    #[error("operand_segment_sizes total {0} does not match the number of operands {1}")]
    OperandSegmentSizesTotalMismatchErr(u32, u32),
}

/// In the case of variadic operands, sometimes it makes sense to group
/// contiguous operands together into a segment. This interface aids doing that.
/// MLIR achieves this by having ODS (tablegen)' `AttrSizedOperandSegments` generate
/// `getODSOperands()` based on the `operandSegmentSizes` attribute.
///
/// ### Attribute(s):
/// | Name | Static Name Identifier | Type |
/// |------|------------------------| -----|
/// | operand_segment_sizes | [ATTR_KEY_OPERAND_SEGMENT_SIZES] | [OperandSegmentSizesAttr](crate::builtin::attributes::OperandSegmentSizesAttr) |
#[op_interface]
pub trait OperandSegmentInterface {
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

    /// Get the `seg_idx`th segment of operands.
    fn get_segment(&self, ctx: &Context, seg_idx: usize) -> Vec<Value> {
        let sizes = self.get_operand_segment_sizes(ctx).0;
        if seg_idx >= sizes.len() {
            return vec![];
        }

        let self_op = self.get_operation().deref(ctx);
        let start = sizes[..seg_idx].iter().sum::<u32>() as usize;
        let len = sizes[seg_idx] as usize;
        self_op.operands().skip(start).take(len).collect()
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
}

/// [Op]s that have exactly one region.
#[op_interface]
pub trait OneRegionInterface: NRegionsInterface<1> {
    /// Get the single region that this [Op] has.
    fn get_region(&self, ctx: &Context) -> Ptr<Region> {
        self.get_operation().deref(ctx).get_region(0)
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
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
    ATTR_KEY_SYM_NAME, "sym_name"
);

#[derive(Error, Debug)]
#[error("Op implementing SymbolOpInterface does not have a symbol defined")]
pub struct SymbolOpInterfaceErr;

/// [Op] that defines or declares a [symbol](https://mlir.llvm.org/docs/SymbolsAndSymbolTables/#symbol).
///
/// ### Attribute(s):
/// | Name | Static Name Identifier | Type |
/// |------|------------------------| -----|
/// | sym_name | [ATTR_KEY_SYM_NAME] | [IdentifierAttr](crate::builtin::attributes::IdentifierAttr) |
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
        let mut seen = FxHashMap::<Identifier, Location>::default();
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
        walk_symbol_table(dyn_clone::clone_box(op), ctx, &mut state, callback);
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
#[error("Expected {0} results, but found {1} results")]
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

#[op_interface]
pub trait OptionalResultInterface: AtMostNResultsInterface<1> {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }

    /// Get the single result defined by this [Op], if any.
    fn get_result(&self, ctx: &Context) -> Option<Value> {
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
pub trait OneResultInterface: NResultsInterface<1> {
    /// Get the single result defined by this [Op].
    fn get_result(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_result(0)
    }

    /// Get the type of the single result defined by this [Op].
    fn result_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        self.get_operation().deref(ctx).get_type(0)
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Expected {} operands, but found {}", .0, .1)]
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

#[op_interface]
pub trait OptionalOpdInterface: AtMostNOpdsInterface<1> {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }

    fn get_operand(&self, ctx: &Context) -> Option<Value> {
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
pub trait OneOpdInterface: NOpdsInterface<1> {
    /// Get the single operand used by this [Op].
    fn get_operand(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_operand(0)
    }

    /// Get the type of the single operand used by this [Op].
    fn operand_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        self.get_operand(ctx).get_type(ctx)
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
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

/// An [Op] with at least one operand, and them all having the same type.
#[op_interface]
pub trait SameOperandsType: AtLeastNOpdsInterface<1> {
    /// Get the common type of the operands.
    fn operand_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        self.get_operation().deref(ctx).get_operand(0).get_type(ctx)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);

        let mut opds = op.operands();
        let ty = opds.next().unwrap().get_type(ctx);
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

#[derive(Error, Debug)]
pub enum OperandNOfTypeError {
    #[error("Op has only {} operands, but expected at least {}", .0, .1)]
    NotEnoughOperands(usize, usize),
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
        let op = op.get_operation().deref(ctx);
        if op.get_num_operands() <= N {
            return verify_err!(
                op.loc(),
                OperandNOfTypeError::NotEnoughOperands(op.get_num_operands(), N)
            );
        }
        let opd_n = op.get_operand(N);
        let opd_n_ty = &*opd_n.get_type(ctx).deref(ctx);
        if !opd_n_ty.as_any().is::<T>() {
            return verify_err!(
                op.loc(),
                OperandNOfTypeError::AllOperandsOfTypeVerifyErr(
                    T::get_type_id_static().disp(ctx).to_string(),
                    opd_n_ty.get_type_id().disp(ctx).to_string()
                )
            );
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Op has different result types")]
pub struct SameResultsTypeVerifyErr;

// An [Op] with at least one result, and them all having the same type.
#[op_interface]
pub trait SameResultsType: AtLeastNResultsInterface<1> {
    /// Get the common type of the results.
    fn result_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        self.get_operation().deref(ctx).get_result(0).get_type(ctx)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);

        let mut results = op.results();
        let ty = results.next().unwrap().get_type(ctx);
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

#[derive(Error, Debug)]
pub enum ResultNOfTypeError {
    #[error("Op has only {} results, but expected at least {}", .0, .1)]
    NotEnoughResults(usize, usize),
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
        let op = op.get_operation().deref(ctx);
        if op.get_num_results() <= N {
            return verify_err!(
                op.loc(),
                ResultNOfTypeError::NotEnoughResults(op.get_num_results(), N)
            );
        }
        let res_n = op.get_result(N);
        let res_n_ty = &*res_n.get_type(ctx).deref(ctx);
        if !res_n_ty.as_any().is::<T>() {
            return verify_err!(
                op.loc(),
                ResultNOfTypeError::AllResultsOfTypeVerifyErr(
                    T::get_type_id_static().disp(ctx).to_string(),
                    res_n_ty.get_type_id().disp(ctx).to_string()
                )
            );
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Op has different operand and result types")]
pub struct SameOperandsAndResultTypeVerifyErr;

/// An [Op] with at least one result and one operand, and them all having the same type.
/// See MLIR's [SameOperandsAndResultType](https://mlir.llvm.org/doxygen/classmlir_1_1OpTrait_1_1SameOperandsAndResultType.html).
#[op_interface]
pub trait SameOperandsAndResultType: SameOperandsType + SameResultsType {
    /// Get the common type of results / operands.
    fn get_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        self.result_type(ctx)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let res_ty = op_cast::<dyn SameResultsType>(op)
            .expect("Op must impl SameResultsType")
            .result_type(ctx);
        let opd_ty = op_cast::<dyn SameOperandsType>(op)
            .expect("Op must impl SameOperandsType")
            .operand_type(ctx);

        if res_ty != opd_ty {
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

dict_key!(ATTR_KEY_CALLEE_TYPE, "callee_type");

/// A call-like op: Transfers control from one function to another.
/// See MLIR's [CallOpInterface](https://mlir.llvm.org/docs/Interfaces/#callinterfaces).
///
/// ### Attribute(s):
///
/// | Name | Static Name Identifier | Type |
/// |------|------------------------| -----|
/// | callee_type | [ATTR_KEY_CALLEE_TYPE] | [TypeAttr](crate::builtin::attributes::TypeAttr) |
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
        if !type_impls::<dyn FunctionTypeInterface>(&**callee_type_attr.get_type(ctx).deref(ctx)) {
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
    fn callee_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        let self_op = self.get_operation().deref(ctx);
        self_op
            .attributes
            .get::<TypeAttr>(&ATTR_KEY_CALLEE_TYPE)
            .unwrap()
            .get_type(ctx)
    }

    /// Set callee type
    fn set_callee_type(&self, ctx: &mut Context, callee_ty: Ptr<TypeObj>) {
        let mut self_op = self.get_operation().deref_mut(ctx);
        let ty_attr = TypeAttr::new(callee_ty);
        self_op
            .attributes
            .set(ATTR_KEY_CALLEE_TYPE.clone(), ty_attr);
    }
}
