//! Utilities to work with [SymbolTableInterface] Ops.

use alloc::{
    boxed::Box,
    string::{String, ToString},
};
use rustc_hash::FxHashMap;
use thiserror::Error;

use crate::{
    arg_err, arg_error,
    builtin::op_interfaces::{SymbolOpInterface, SymbolTableInterface},
    context::{Context, Ptr},
    graph::walkers::{
        IRNode, WALKCONFIG_PREORDER_FORWARD,
        interruptible::{WalkResult, immutable::walk_region, walk_advance, walk_skip},
    },
    identifier::Identifier,
    irbuild::{
        inserter::{IRInserter, Inserter, OpInsertionPoint},
        listener::DummyListener,
    },
    linked_list::ContainsLinkedList,
    location::Located,
    op::op_cast,
    operation::Operation,
    printable::Printable,
    result::Result,
};

/// A utility to efficiently lookup and update [Symbol](SymbolOpInterface)s
/// in a [SymbolTableInterface] Op. Similar to its counterpart in MLIR,
/// inserting into and erasing from this [SymbolTable] will also insert and
/// erase from the Operation given to it at construction.
pub struct SymbolTable {
    symbol_table_op: Box<dyn SymbolTableInterface>,
    symbol_table: FxHashMap<Identifier, Box<dyn SymbolOpInterface>>,
}

#[derive(Error, Debug)]

pub enum SymbolTableErr {
    #[error("Op does not implement SymbolTableInterface")]
    DoesNotImplementSymbolTableInterface,
    #[error("Symbol redefined: {0}")]
    SymbolRedefined(String),
    #[error("Op does not implement SymbolOpInterface")]
    DoesNotImplementSymbolOpInterface,
    #[error("Symbol Op cannot be inserted as it is already in another symbol table")]
    SymbolOpAlreadyInAnotherTable(String),
    #[error("Insertion point must already be inside the same symbol table")]
    InsertionPointNotInSameTable,
    #[error("Symbol op does not belong to the symbol table op")]
    SymbolOpNotInTableOp,
}

impl SymbolTable {
    /// Create a new [SymbolTable] from a [SymbolTableInterface] Op.
    pub fn new(ctx: &Context, symbol_table_op: Box<dyn SymbolTableInterface>) -> Self {
        // Go through the all operations in symbol_table_op and build a table.
        let mut symbol_table = FxHashMap::<Identifier, Box<dyn SymbolOpInterface>>::default();
        let table_ops_block = symbol_table_op.get_body(ctx, 0);
        for op in table_ops_block.deref(ctx).iter(ctx) {
            if let Some(sym_op) =
                op_cast::<dyn SymbolOpInterface>(Operation::get_op_dyn(op, ctx).as_ref())
            {
                let sym = sym_op.get_symbol_name(ctx);
                if let Some(prev_op) =
                    symbol_table.insert(sym.clone(), dyn_clone::clone_box(sym_op))
                {
                    // It's the job of the verifier to have caught this. So we panic.
                    panic!(
                        "Symbol {} defined at {} was previously defined at {}",
                        sym,
                        symbol_table_op.loc(ctx).disp(ctx),
                        prev_op.loc(ctx).disp(ctx)
                    );
                }
            }
        }

        Self {
            symbol_table_op,
            symbol_table,
        }
    }

    /// Insert a new [Symbol](SymbolOpInterface) into the [SymbolTable].
    /// If an insertion point is provided, that point must be inside self.symbol_table_op.
    /// If no insertion point is provided, the symbol op will be inserted at the end
    /// (before the terminator if one exists). symbol_op's parent must be either `None`,
    /// or the same as self.symbol_table_op.
    pub fn insert(
        &mut self,
        ctx: &Context,
        symbol_op: Box<dyn SymbolOpInterface>,
        insert_pt: Option<OpInsertionPoint>,
    ) -> Result<()> {
        let symbol_name = symbol_op.get_symbol_name(ctx);
        if let Some(prev_op) = self
            .symbol_table
            .insert(symbol_name.clone(), symbol_op.clone())
        {
            return arg_err!(
                symbol_op.loc(ctx),
                arg_error!(
                    prev_op.loc(ctx),
                    SymbolTableErr::SymbolRedefined(symbol_name.to_string())
                )
            );
        }

        let symbol_table_op = self.symbol_table_op.get_operation();
        let symbol_op = symbol_op.get_operation();

        if let Some(symbol_op_parent) = symbol_op.deref(ctx).get_parent_op(ctx) {
            if symbol_table_op != symbol_op_parent {
                return arg_err!(
                    symbol_op.deref(ctx).loc(),
                    SymbolTableErr::SymbolOpAlreadyInAnotherTable(symbol_name.to_string())
                );
            }
            // Nothing more to do.
        } else if let Some(insert_pt) = insert_pt {
            let insert_pt_parent = insert_pt
                .get_insertion_block(ctx)
                .expect("Invalid insertion point")
                .deref(ctx)
                .get_parent_op(ctx);
            match insert_pt_parent {
                Some(parent) if parent == symbol_table_op => {
                    let mut inserter = IRInserter::<DummyListener>::new(insert_pt);
                    inserter.insert_operation(ctx, symbol_op);
                }
                _ => {
                    return arg_err!(
                        symbol_op.deref(ctx).loc(),
                        SymbolTableErr::InsertionPointNotInSameTable
                    );
                }
            }
        } else {
            // Insert at the end, before the terminator if one exists.
            let block = self.symbol_table_op.get_body(ctx, 0);
            let terminator = block.deref(ctx).get_terminator(ctx);
            match terminator {
                Some(terminator) => symbol_op.insert_before(ctx, terminator),
                None => symbol_op.insert_at_back(block, ctx),
            }
        }

        Ok(())
    }

    /// Remove `symbol_op` from this [SymbolTable].
    /// The operation is unlinked from its parent, but is not erased.
    pub fn remove(&mut self, ctx: &Context, symbol_op: Box<dyn SymbolOpInterface>) -> Result<()> {
        let symbol_op_opr = symbol_op.get_operation();
        if !matches!(symbol_op_opr.deref(ctx).get_parent_op(ctx), Some(parent_op) if parent_op == self.symbol_table_op.get_operation())
        {
            return arg_err!(symbol_op.loc(ctx), SymbolTableErr::SymbolOpNotInTableOp);
        }

        let symbol = symbol_op.get_symbol_name(ctx);
        self.symbol_table.remove(&symbol);

        symbol_op_opr.unlink(ctx);

        Ok(())
    }

    /// Remove `symbol_op` from this [SymbolTable] and erase it.
    /// This will panic if `symbol_op` has SSA uses.
    pub fn erase(
        &mut self,
        ctx: &mut Context,
        symbol_op: Box<dyn SymbolOpInterface>,
    ) -> Result<()> {
        self.remove(ctx, symbol_op.clone())?;
        Operation::erase(symbol_op.get_operation(), ctx);
        Ok(())
    }

    /// Get the [Symbol](SymbolOpInterface) with the given name.
    pub fn lookup(&self, symbol_name: &Identifier) -> Option<Box<dyn SymbolOpInterface>> {
        self.symbol_table.get(symbol_name).cloned()
    }
}

pub type SymbolTableWalkerCallback<State> =
    fn(&Context, &mut State, Ptr<Operation>) -> WalkResult<()>;

/// Walk all (nested) operations within the region of `symbol_table_op`,
/// without traversing into any nested symbol tables.
pub fn walk_symbol_table<State>(
    symbol_table_op: Box<dyn SymbolTableInterface>,
    ctx: &Context,
    state: &mut State,
    callback: SymbolTableWalkerCallback<State>,
) {
    struct StateWithCallback<'a, State> {
        callback: SymbolTableWalkerCallback<State>,
        state: &'a mut State,
    }

    fn skip_nested_symbol_tables_callback<State>(
        ctx: &Context,
        state: &mut StateWithCallback<State>,
        node: IRNode,
    ) -> WalkResult<()> {
        if let IRNode::Operation(opr) = node {
            let op = Operation::get_op_dyn(opr, ctx);
            if op_cast::<dyn SymbolTableInterface>(op.as_ref()).is_some() {
                return walk_skip();
            }
            (state.callback)(ctx, state.state, opr)
        } else {
            walk_advance()
        }
    }

    let _ = walk_region(
        ctx,
        &mut StateWithCallback { callback, state },
        &WALKCONFIG_PREORDER_FORWARD,
        symbol_table_op.get_region(ctx),
        skip_nested_symbol_tables_callback,
    );
}

/// A collection of [SymbolTable]s, to simplify algorithms that run
/// recursively on nested [SymbolTable]s. The actual tables are constructed lazily.
/// See MLIR's [SymbolTableCollection](https://mlir.llvm.org/doxygen/classmlir_1_1SymbolTableCollection.html).
#[derive(Default)]
pub struct SymbolTableCollection {
    symbol_tables: FxHashMap<Ptr<Operation>, SymbolTable>,
}

/// Returns the nearest symbol table from a given operation `from`
pub fn nearest_symbol_table(
    ctx: &Context,
    from: Ptr<Operation>,
) -> Option<Box<dyn SymbolTableInterface>> {
    let mut op = from;
    loop {
        if let Some(symbol_table) =
            op_cast::<dyn SymbolTableInterface>(Operation::get_op_dyn(op, ctx).as_ref())
        {
            return Some(dyn_clone::clone_box(symbol_table));
        }
        if let Some(parent) = op.deref(ctx).get_parent_op(ctx) {
            op = parent;
        } else {
            return None;
        }
    }
}

impl SymbolTableCollection {
    /// Create a new empty [SymbolTableCollection].
    pub fn new() -> Self {
        Self::default()
    }

    /// Lookup symbol in the given [SymbolTableInterface] Op.
    /// A symbol table is computed on demand and cached in the collection.
    pub fn lookup_symbol_in_table(
        &mut self,
        ctx: &Context,
        symbol_table_op: Box<dyn SymbolTableInterface>,
        symbol_name: &Identifier,
    ) -> Option<Box<dyn SymbolOpInterface>> {
        self.get_symbol_table(ctx, symbol_table_op)
            .lookup(symbol_name)
    }

    /// Get the symbol table for the symbol table operation, constructing it if necessary.
    pub fn get_symbol_table(
        &mut self,
        ctx: &Context,
        symbol_table_op: Box<dyn SymbolTableInterface>,
    ) -> &mut SymbolTable {
        self.symbol_tables
            .entry(symbol_table_op.get_operation())
            .or_insert_with(|| SymbolTable::new(ctx, symbol_table_op))
    }

    /// Returns the [SymbolOpInterface] Op registered with the given symbol within the
    /// closest [SymbolTableInterface] parent Op of, or including, 'from'.
    pub fn lookup_symbol_in_nearest_table(
        &mut self,
        ctx: &Context,
        from: Ptr<Operation>,
        symbol_name: &Identifier,
    ) -> Option<Box<dyn SymbolOpInterface>> {
        let symbol_table = nearest_symbol_table(ctx, from)?;
        self.lookup_symbol_in_table(ctx, symbol_table, symbol_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        builtin::{
            ops::{FuncOp, ModuleOp},
            types::FunctionType,
        },
        context::Context,
        identifier::Identifier,
        op::Op,
    };
    use alloc::vec;

    #[test]
    fn test_symbol_table() -> Result<()> {
        let ctx = &mut Context::new();
        let module_op = ModuleOp::new(ctx, Identifier::try_from("module").unwrap());
        let mut symbol_table = SymbolTable::new(ctx, Box::new(module_op));

        let fty = FunctionType::get(ctx, vec![], vec![]);
        let f1 = FuncOp::new(ctx, "f1".try_into().unwrap(), fty);

        symbol_table.insert(ctx, Box::new(f1), None)?;
        assert!(
            f1.get_operation().deref(ctx).get_parent_op(ctx) == Some(module_op.get_operation())
        );
        let f1_lookedup = symbol_table.lookup(&"f1".try_into().unwrap()).unwrap();
        assert!(f1_lookedup.get_operation() == f1.get_operation());

        let f2 = FuncOp::new(ctx, "f2".try_into().unwrap(), fty);
        symbol_table.insert(
            ctx,
            Box::new(f2),
            Some(OpInsertionPoint::BeforeOperation(f1.get_operation())),
        )?;
        assert!(
            f2.get_operation().deref(ctx).get_parent_op(ctx) == Some(module_op.get_operation())
        );
        let f2_lookedup = symbol_table.lookup(&"f2".try_into().unwrap()).unwrap();
        assert!(f2_lookedup.get_operation() == f2.get_operation());

        symbol_table.remove(ctx, Box::new(f1))?;
        assert!(symbol_table.lookup(&"f1".try_into().unwrap()).is_none());
        assert!(f1.get_operation().deref(ctx).get_parent_op(ctx).is_none());

        Ok(())
    }

    #[test]
    #[should_panic(expected = "Dangling Ptr deref")]
    fn test_symbol_table_erase() {
        let ctx = &mut Context::new();
        let module_op = ModuleOp::new(ctx, Identifier::try_from("module").unwrap());
        let mut symbol_table = SymbolTable::new(ctx, Box::new(module_op));

        let fty = FunctionType::get(ctx, vec![], vec![]);
        let f1 = FuncOp::new(ctx, "f1".try_into().unwrap(), fty);

        symbol_table.insert(ctx, Box::new(f1), None).unwrap();
        assert!(
            f1.get_operation().deref(ctx).get_parent_op(ctx) == Some(module_op.get_operation())
        );
        let f1_lookedup = symbol_table.lookup(&"f1".try_into().unwrap()).unwrap();
        assert!(f1_lookedup.get_operation() == f1.get_operation());

        symbol_table.erase(ctx, Box::new(f1)).unwrap();
        f1.get_operation().deref(ctx);
    }

    #[test]
    fn test_symbol_table_collection() -> Result<()> {
        let ctx = &mut Context::new();

        let module_op = ModuleOp::new(ctx, Identifier::try_from("module").unwrap());
        let nested_module_op = ModuleOp::new(ctx, Identifier::try_from("nested_module").unwrap());
        let mut symbol_table_collection = SymbolTableCollection::new();

        let fty = FunctionType::get(ctx, vec![], vec![]);
        let f1 = FuncOp::new(ctx, "f1".try_into().unwrap(), fty);

        symbol_table_collection
            .get_symbol_table(ctx, Box::new(module_op))
            .insert(ctx, Box::new(f1), None)?;

        let f1_lookedup = symbol_table_collection
            .lookup_symbol_in_table(ctx, Box::new(module_op), &"f1".try_into().unwrap())
            .unwrap();
        assert!(f1_lookedup.get_operation() == f1.get_operation());

        let f2 = FuncOp::new(ctx, "f2".try_into().unwrap(), fty);
        symbol_table_collection
            .get_symbol_table(ctx, Box::new(nested_module_op))
            .insert(ctx, Box::new(f2), None)?;

        let f2_lookedup = symbol_table_collection
            .lookup_symbol_in_table(ctx, Box::new(nested_module_op), &"f2".try_into().unwrap())
            .unwrap();

        assert!(f2_lookedup.get_operation() == f2.get_operation());

        symbol_table_collection
            .get_symbol_table(ctx, Box::new(module_op))
            .insert(ctx, Box::new(nested_module_op), None)?;

        let nested_module_lookedup = symbol_table_collection
            .lookup_symbol_in_nearest_table(
                ctx,
                f1.get_operation(),
                &"nested_module".try_into().unwrap(),
            )
            .unwrap();
        assert!(nested_module_lookedup.get_operation() == nested_module_op.get_operation());

        let f1_lookedup = symbol_table_collection
            .lookup_symbol_in_nearest_table(ctx, f1.get_operation(), &"f1".try_into().unwrap())
            .unwrap();
        assert!(f1_lookedup.get_operation() == f1.get_operation());

        Ok(())
    }
}
