//! A pass manager and analysis framework.
//!
//! This module provides:
//! 1. [`Pass`]: A transformation that runs on an operation.
//! 2. [`PassManager`]: Runs a pipeline of nested passes on itself and
//!    each immediately nested operation.
//! 3. [`GuardedPass`], [`OpPass`], and [`OpInterfacePass`]: Wrappers that
//!    constrain where a pass is allowed to run.
//! 4. [`Analysis`] and [`AnalysisManager`]: Provides analyses caching with
//!    preservation and invalidation support.
//!
//! # Usage
//!
//! A pass receives three inputs:
//! * The operation it should process,
//! * A mutable reference to the context
//! * An analysis cache.
//!
//! It returns a [`PassResult`] indicating whether IR changed and which analyses
//! are preserved.
//!
//! If a pass reports [`IRStatus::Unchanged`], all analyses are treated as
//! preserved. If it reports changes, analyses not explicitly preserved are
//! invalidated.
//!
//! **NOTE**: A pass must not modify the IR outside of the operation it is applied to.
//!
//! ## Example: Define and run a simple pass
//!
//! ```rust
//! use pliron::{
//!     context::Context,
//!     operation::Operation,
//!     pass_manager::{AnalysisManager, Pass, PassResult, PassManager},
//!     result::Result,
//!     irbuild::IRStatus,
//! };
//!
//! #[derive(Default)]
//! struct NoOpPass;
//!
//! impl Pass for NoOpPass {
//!     fn name(&self) -> &str { "noop" }
//!
//!     fn run(
//!         &mut self,
//!         _op: pliron::context::Ptr<Operation>,
//!         _ctx: &mut Context,
//!         _analyses: &mut AnalysisManager,
//!     ) -> Result<PassResult> {
//!         let mut result = PassResult::default();
//!         result.ir_changed = IRStatus::Unchanged;
//!         Ok(result)
//!     }
//! }
//!
//! fn run_pipeline(
//!     root: pliron::context::Ptr<Operation>,
//!     ctx: &mut Context,
//! ) -> Result<()> {
//!     let mut pm = PassManager::default();
//!     pm.add_pass(NoOpPass);
//!     let _ = pm.run(root, ctx, &mut AnalysisManager::default())?;
//!     Ok(())
//! }
//! ```
//!
//! ## Example: Restrict a pass to a specific op kind.
//! [OpPassManager] is a convenient wrapper around [GuardedPass] that allows
//! you to run a pass only on operations of a specific [Op]. Similarly, [OpPass]
//! allows you to run any pass on operations of a specific [Op].
//!
//! ```rust
//! use pliron::{
//!     context::Context,
//!     irbuild::IRStatus,
//!     operation::Operation,
//!     pass_manager::{AnalysisManager, PassGroup, OpPass, OpPassManager, Pass, PassResult},
//!     result::Result,
//! };
//! use pliron::builtin::ops::{FuncOp, ModuleOp};
//!
//! #[derive(Default)]
//! struct MyFuncPass;
//!
//! impl Pass for MyFuncPass {
//!     fn name(&self) -> &str { "my_func_pass" }
//!
//!     fn run(
//!         &mut self,
//!         _op: pliron::context::Ptr<Operation>,
//!         _ctx: &mut Context,
//!         _analyses: &mut AnalysisManager,
//!     ) -> Result<PassResult> {
//!         let mut result = PassResult::default();
//!         result.ir_changed = IRStatus::Unchanged;
//!         Ok(result)
//!     }
//! }
//!
//! // Run a pass manager only when the root op is ModuleOp.
//! let mut module_pm = OpPassManager::<ModuleOp>::default();
//! // Add a pass that runs only on nested FuncOp operations.
//! module_pm.add_pass(OpPass::<MyFuncPass, FuncOp>::default());
//! ```
//!
//! ## Example: Analysis caching and preservation
//!
//! ```rust
//! use pliron::{
//!     context::Context,
//!     operation::Operation,
//!     pass_manager::{Analysis, AnalysisManager, Pass, PassResult},
//!     result::Result,
//! };
//!
//! struct MyAnalysis;
//!
//! impl Analysis for MyAnalysis {
//!     fn name(&self) -> &str { "my_analysis" }
//!     fn compute(
//!         _op: pliron::context::Ptr<Operation>,
//!         _ctx: &Context,
//!         _analyses: &mut AnalysisManager,
//!     ) -> Result<Self> {
//!         Ok(Self)
//!     }
//! }
//!
//! struct UsesMyAnalysis;
//!
//! impl Pass for UsesMyAnalysis {
//!     fn name(&self) -> &str { "uses_my_analysis" }
//!
//!     fn run(
//!         &mut self,
//!         op: pliron::context::Ptr<Operation>,
//!         ctx: &mut Context,
//!         analyses: &mut AnalysisManager,
//!     ) -> Result<PassResult> {
//!         let _analysis = analyses.get_analysis::<MyAnalysis>(op, ctx)?;
//!         let mut result = PassResult::default();
//!         // If this pass mutates IR but does not invalidate MyAnalysis,
//!         // explicitly preserve it.
//!         result.set_preserved::<MyAnalysis>();
//!         Ok(result)
//!     }
//! }
//! ```

use core::cell::{Ref, RefCell, RefMut};

use alloc::{boxed::Box, vec::Vec};
use downcast_rs::{Downcast, impl_downcast};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    context::{Context, Ptr},
    irbuild::IRStatus,
    op::{Op, OpInterfaceMarker, op_impls},
    operation::Operation,
    result::Result,
};

#[derive(Default)]
/// The result of running a [Pass].
///
/// 1. [IRStatus]: Whether the IR was changed or not.
/// 2. A list of preserved analyses.
///
/// [IRStatus::Unchanged] implies all analyses are preserved.
pub struct PassResult {
    pub ir_changed: IRStatus,
    preserved_analyses: FxHashSet<core::any::TypeId>,
}

impl PassResult {
    pub fn set_preserved<A: Analysis + 'static>(&mut self) {
        self.preserved_analyses.insert(core::any::TypeId::of::<A>());
    }
}

/// A pass is any code that runs on an [Operation].
/// Typically a transformation or a nested pass manager.
///
/// Transformations must not modify the IR outside of the [Operation] they are applied to.
pub trait Pass {
    /// Name of the pass.
    fn name(&self) -> &str;
    /// Run the pass and return whether the IR changed and which analyses are preserved.
    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult>;
}

/// A [Pass] that contains a group of nested passes.
pub trait PassGroup: Pass {
    fn add_pass(&mut self, pass: impl Pass + 'static);
}

#[derive(Default)]
/// A [Pass] that manages its [PassGroup].
/// i.e., it runs the passes in the group on itself and each immediately nested operation.
pub struct PassManager {
    passes: Vec<Box<dyn Pass>>,
}

impl Pass for PassManager {
    fn name(&self) -> &str {
        "pass_manager"
    }

    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        use crate::linked_list::ContainsLinkedList;

        let mut pass_res = PassResult::default();

        // Run each pass in the group on the current operation.
        for pass in &mut self.passes {
            let res = pass.run(op, ctx, analyses)?;
            pass_res.ir_changed |= res.ir_changed;
            // Invalidate analyses that are not preserved.
            analyses.retain_preserved(&res);
        }

        let regions = op.deref(ctx).regions().collect::<Vec<_>>();
        for region in regions {
            let blocks = region.deref(ctx).iter(ctx).collect::<Vec<_>>();
            for block in blocks {
                let ops = block.deref(ctx).iter(ctx).collect::<Vec<_>>();
                for nested_op in ops {
                    for pass in &mut self.passes {
                        let res = pass.run(nested_op, ctx, analyses)?;
                        pass_res.ir_changed |= res.ir_changed;
                        // Invalidate analyses that are not preserved.
                        analyses.retain_preserved(&res);
                    }
                }
            }
        }

        // Since we invalidate analyses after each pass,
        // all remaining analyses are preserved.
        let preserved_analyses = analyses.list_analyses();
        pass_res.preserved_analyses = preserved_analyses;

        Ok(pass_res)
    }
}

impl PassManager {
    /// Add a [Pass] to the pipeline.
    pub fn add_pass(&mut self, pass: impl Pass + 'static) {
        self.passes.push(Box::new(pass));
    }
}

impl PassGroup for PassManager {
    fn add_pass(&mut self, pass: impl Pass + 'static) {
        self.add_pass(pass);
    }
}

/// A `Guard` determines whether a [Pass] is applicable to a given [Operation].
pub trait Guard {
    /// Applicability of a [Pass] for a given [Operation].
    fn is_allowed(&self, op: Ptr<Operation>, ctx: &Context) -> bool;
}

/// Allow [Operation]s of a specific `Op`.
pub struct OpGuard<T: Op> {
    _marker: core::marker::PhantomData<T>,
}

impl<T: Op> Default for OpGuard<T> {
    fn default() -> Self {
        Self {
            _marker: core::marker::PhantomData,
        }
    }
}

impl<T: Op> Guard for OpGuard<T> {
    fn is_allowed(&self, op: Ptr<Operation>, ctx: &Context) -> bool {
        Operation::is_op::<T>(op, ctx)
    }
}

/// Allow [Operation]s that implement a specific `OpInterface`.
pub struct OpInterfaceGuard<T: ?Sized + OpInterfaceMarker + 'static> {
    _marker: core::marker::PhantomData<T>,
}

impl<T: ?Sized + OpInterfaceMarker + 'static> Default for OpInterfaceGuard<T> {
    fn default() -> Self {
        Self {
            _marker: core::marker::PhantomData,
        }
    }
}

impl<T: ?Sized + OpInterfaceMarker + 'static> Guard for OpInterfaceGuard<T> {
    fn is_allowed(&self, op: Ptr<Operation>, ctx: &Context) -> bool {
        let op = Operation::get_op_dyn(op, ctx);
        op_impls::<T>(&*op)
    }
}

/// Adds a [Guard] to a [Pass], making it run only on [Operation]s that the [Guard] allows.
#[derive(Default)]
pub struct GuardedPass<P: Pass, G: Guard> {
    pass: P,
    guard: G,
}

impl<P: Pass, G: Guard> GuardedPass<P, G> {
    pub fn new(pass: P, guard: G) -> Self {
        Self { pass, guard }
    }
}

impl<P: Pass, G: Guard> Pass for GuardedPass<P, G> {
    fn name(&self) -> &str {
        self.pass.name()
    }

    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        if self.guard.is_allowed(op, ctx) {
            self.pass.run(op, ctx, analyses)
        } else {
            Ok(PassResult::default())
        }
    }
}

impl<P: Pass + PassGroup, G: Guard> PassGroup for GuardedPass<P, G> {
    fn add_pass(&mut self, pass: impl Pass + 'static) {
        self.pass.add_pass(pass);
    }
}

/// A [GuardedPass] that allows [Operation]s of a specific [Op].
pub type OpPass<P, T> = GuardedPass<P, OpGuard<T>>;

/// A [GuardedPass] that allows [Operation]s that implement a specific `OpInterface`.
pub type OpInterfacePass<P, T> = GuardedPass<P, OpInterfaceGuard<T>>;

/// A [PassManager] that runs on [Operation]s of a specific [Op].
pub type OpPassManager<T> = OpPass<PassManager, T>;

/// A [PassManager] that runs on [Operation]s that implement a specific `OpInterface`.
pub type OpInterfacePassManager<T> = OpInterfacePass<PassManager, T>;

/// An analysis is any code that computes information
/// about an [Operation] without modifying the IR.
pub trait Analysis: Downcast {
    /// Name of this analysis.
    fn name(&self) -> &str;
    /// Compute this analysis for a given [Operation].
    fn compute(op: Ptr<Operation>, ctx: &Context, analyses: &mut AnalysisManager) -> Result<Self>
    where
        Self: Sized;
}
impl_downcast!(Analysis);

/// An [Analysis] together with the [Operation] it is computed for.
/// Used as a key in the [AnalysisManager] cache.
type AnalysisManagerKey = (core::any::TypeId, Ptr<Operation>);

#[derive(Default)]
/// A manager for analyses, responsible for caching and invalidating them.
pub struct AnalysisManager {
    analyses: FxHashMap<AnalysisManagerKey, Box<RefCell<dyn Analysis>>>,
}

impl AnalysisManager {
    /// Compute (if not already cached) and cache an analysis `A` for [Operation] `op`.
    pub fn compute_analysis<A: Analysis + 'static>(
        &mut self,
        op: Ptr<Operation>,
        ctx: &Context,
    ) -> Result<()> {
        let key = (core::any::TypeId::of::<A>(), op);
        if !self.analyses.contains_key(&key) {
            let analysis = A::compute(op, ctx, self)?;
            self.analyses.insert(key, Box::new(RefCell::new(analysis)));
        }
        Ok(())
    }

    /// Get [RefMut] for analysis `A`, computing it if not cached.
    pub fn get_analysis_mut<'a, A: Analysis + 'static>(
        &'a mut self,
        op: Ptr<Operation>,
        ctx: &Context,
    ) -> Result<RefMut<'a, A>> {
        self.compute_analysis::<A>(op, ctx)?;
        let key = (core::any::TypeId::of::<A>(), op);
        let analysis = self.analyses.get(&key).unwrap();
        Ok(RefMut::map(analysis.borrow_mut(), |a| {
            a.downcast_mut::<A>().unwrap()
        }))
    }

    /// Get [Ref] for analysis `A`, computing it if not cached.
    pub fn get_analysis<'a, A: Analysis + 'static>(
        &'a mut self,
        op: Ptr<Operation>,
        ctx: &Context,
    ) -> Result<Ref<'a, A>> {
        self.compute_analysis::<A>(op, ctx)?;
        let key = (core::any::TypeId::of::<A>(), op);
        let analysis = self.analyses.get(&key).unwrap();
        Ok(Ref::map(analysis.borrow(), |a| {
            a.downcast_ref::<A>().unwrap()
        }))
    }

    /// Get, if cached, [Ref] for analysis `A`.
    pub fn try_get_analysis<'a, A: Analysis + 'static>(
        &'a self,
        op: Ptr<Operation>,
    ) -> Option<Ref<'a, A>> {
        let key = (core::any::TypeId::of::<A>(), op);
        self.analyses
            .get(&key)
            .map(|analysis| Ref::map(analysis.borrow(), |a| a.downcast_ref::<A>().unwrap()))
    }

    /// Get, if cached, [RefMut] for analysis `A`.
    pub fn try_get_analysis_mut<'a, A: Analysis + 'static>(
        &'a self,
        op: Ptr<Operation>,
    ) -> Option<RefMut<'a, A>> {
        let key = (core::any::TypeId::of::<A>(), op);
        self.analyses
            .get(&key)
            .map(|analysis| RefMut::map(analysis.borrow_mut(), |a| a.downcast_mut::<A>().unwrap()))
    }

    /// Retain only analyses that are preserved by a [PassResult].
    pub fn retain_preserved(&mut self, pass_res: &PassResult) {
        if pass_res.ir_changed == IRStatus::Unchanged {
            return;
        }
        self.analyses
            .retain(|(type_id, _), _| pass_res.preserved_analyses.contains(type_id));
    }

    /// Get a list of all analyses currently cached.
    fn list_analyses(&self) -> FxHashSet<core::any::TypeId> {
        self.analyses.keys().map(|(type_id, _)| *type_id).collect()
    }
}
