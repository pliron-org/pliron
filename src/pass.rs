//! A framework to run passes and manage analyses.
//!
//! The design is centered around the [Pass] trait and aims to be
//! flexible and composable. Passes can be combined into pipelines,
//! and analyses can be cached and invalidated between passes.
//!
//! This module provides:
//! 1. [`Pass`]: A transformation that runs on an operation.
//! 2. [`Passes`]: Runs a sequence of [Pass]es on the provided operation,
//!    managing invalidation of analyses between passes.
//! 3. [`NestedOpsPass`]: Runs a provided [Pass] on each immediately nested operation,
//!    managing invalidation of analyses between runs.
//! 4. [`GuardedPass`], [`OpPass`], and [`OpInterfacePass`]: Wrappers that
//!    constrain where a pass is allowed to run.
//! 5. [`Analysis`] and [`AnalysisManager`]: Provides analyses caching with
//!    preservation and invalidation support.
//! 6. [`PMConfig`] provides configuration that can be set via the [AnalysisManager].
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
//!     pass::{AnalysisManager, Pass, PassResult, Passes},
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
//!     let mut passes = Passes::default();
//!     passes.add_pass(NoOpPass);
//!     let res = passes.run(root, ctx, &mut AnalysisManager::default())?;
//!     assert!(matches!(res.ir_changed, IRStatus::Unchanged));
//!     Ok(())
//! }
//! ```
//!
//! ## Example: Restrict a pass to a specific op kind.
//! [GuardedPass] is a [Pass] wrapper restricting the run to specific operations.
//! [OpPass] is a [GuardedPass] that restricts the run to a specific [Op].
//! [NestedOpsPass] is a [Pass] that runs a provided [Pass] on each immediately nested operation.
//!
//! ```rust
//! use pliron::{
//!     context::Context,
//!     irbuild::IRStatus,
//!     operation::Operation,
//!     pass::{AnalysisManager, NestedOpsPass, Passes, OpPass, Pass, PassResult},
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
//! let mut passes = OpPass::<ModuleOp, Passes>::default();
//! // Add a pass that runs only on nested FuncOp operations.
//! let nested_pass = NestedOpsPass::new(OpPass::<FuncOp, MyFuncPass>::default());
//! passes.add_pass(nested_pass);
//! ```
//!
//! ## Example: Analysis caching and preservation
//!
//! ```rust
//! use pliron::{
//!     context::Context,
//!     operation::Operation,
//!     pass::{Analysis, AnalysisManager, Pass, PassResult},
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

use core::{
    cell::{Ref, RefCell, RefMut},
    ops::{Deref, DerefMut},
};

use alloc::{boxed::Box, string::String, vec::Vec};
use downcast_rs::{Downcast, impl_downcast};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    context::{Context, Ptr},
    identifier::Identifier,
    irbuild::IRStatus,
    op::{Op, OpInterfaceMarker, op_impls},
    operation::{OpDbg, Operation, verify_operation},
    printable::Printable,
    result::Result,
    utils::timer::Timer,
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

/// A pass is any code that runs on the provided [Operation].
/// Typically a transformation or (nested) passes.
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

    /// If this [Pass] contains, manages and runs other passes,
    /// get [self] as a [PassManager].
    /// Most passes do not qualify and must not override this method.
    fn as_pass_manager(&mut self) -> Option<&mut dyn PassManager> {
        None
    }
}

#[derive(Default)]
/// Runs a sequence of [Pass]es on the provided [Operation].
/// Manages invalidation of analyses between passes.
pub struct Passes {
    passes: Vec<Box<dyn Pass>>,
}

impl Pass for Passes {
    fn name(&self) -> &str {
        "passes"
    }

    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let mut pass_res = PassResult::default();

        // Run each pass in the list on the current operation.
        for pass in &mut self.passes {
            let res = pass.run(op, ctx, analyses)?;
            pass_res.ir_changed |= res.ir_changed;
            // Invalidate analyses that are not preserved.
            analyses.retain_preserved(&res);
        }

        // Since we invalidate analyses after each pass,
        // all remaining analyses are preserved.
        let preserved_analyses = analyses.list_analyses();
        pass_res.preserved_analyses = preserved_analyses;

        Ok(pass_res)
    }

    fn as_pass_manager(&mut self) -> Option<&mut dyn PassManager> {
        Some(self)
    }
}

impl Passes {
    /// Add a [Pass] to the list of passes to run.
    pub fn add_pass(&mut self, pass: impl Pass + 'static) {
        self.passes.push(Box::new(pass));
    }
}

impl PassManager for Passes {}

/// Runs a provided [Pass] on each immediately nested [Operation].
/// Manages invalidation of analyses between runs.
pub struct NestedOpsPass {
    pass: Box<dyn Pass>,
}

impl Pass for NestedOpsPass {
    fn name(&self) -> &str {
        "nested_ops_pass"
    }

    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        use crate::linked_list::ContainsLinkedList;

        let mut pass_res = PassResult::default();

        let regions = op.deref(ctx).regions().collect::<Vec<_>>();
        for region in regions {
            let blocks = region.deref(ctx).iter(ctx).collect::<Vec<_>>();
            for block in blocks {
                let ops = block.deref(ctx).iter(ctx).collect::<Vec<_>>();
                for nested_op in ops {
                    let res = self.pass.run(nested_op, ctx, analyses)?;
                    pass_res.ir_changed |= res.ir_changed;
                    // Invalidate analyses that are not preserved.
                    analyses.retain_preserved(&res);
                }
            }
        }

        // Since we invalidate analyses after each pass,
        // all remaining analyses are preserved.
        let preserved_analyses = analyses.list_analyses();
        pass_res.preserved_analyses = preserved_analyses;

        Ok(pass_res)
    }

    fn as_pass_manager(&mut self) -> Option<&mut dyn PassManager> {
        Some(self)
    }
}

impl NestedOpsPass {
    pub fn new(pass: impl Pass + 'static) -> Self {
        Self {
            pass: Box::new(pass),
        }
    }
}

impl PassManager for NestedOpsPass {}

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
pub struct GuardedPass<G: Guard, P: Pass> {
    guard: G,
    pass: P,
}

impl<G: Guard, P: Pass> GuardedPass<G, P> {
    pub fn new(guard: G, pass: P) -> Self {
        Self { guard, pass }
    }
}

impl<G: Guard, P: Pass> PassManager for GuardedPass<G, P> {}

impl<G: Guard, P: Pass> Pass for GuardedPass<G, P> {
    fn name(&self) -> &str {
        "guarded_pass"
    }

    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        if self.guard.is_allowed(op, ctx) {
            <Self as PassManager>::run_pass(&mut self.pass, op, ctx, analyses)
        } else {
            Ok(PassResult::default())
        }
    }

    fn as_pass_manager(&mut self) -> Option<&mut dyn PassManager> {
        Some(self)
    }
}

impl<G: Guard, P: Pass> Deref for GuardedPass<G, P> {
    type Target = P;

    fn deref(&self) -> &Self::Target {
        &self.pass
    }
}

impl<G: Guard, P: Pass> DerefMut for GuardedPass<G, P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.pass
    }
}

/// A [GuardedPass] that allows [Operation]s of a specific [Op].
pub type OpPass<T, P> = GuardedPass<OpGuard<T>, P>;

/// A [GuardedPass] that allows [Operation]s that implement a specific `OpInterface`.
pub type OpInterfacePass<T, P> = GuardedPass<OpInterfaceGuard<T>, P>;

/// A [Pass] that contains, manages and runs other [Pass]es.
/// The only requirement (that cannot be enforced by the type system)
/// is that a [PassManager] [Pass] must run its contained [Passes] via
/// [PassManager::run_pass].
pub trait PassManager {
    /// Run a [Pass], calling pre/post hooks for non-manager passes.
    fn run_pass(
        pass: &mut dyn Pass,
        op: Ptr<Operation>,
        ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult>
    where
        Self: Sized,
    {
        let is_pass_manager = pass.as_pass_manager().is_some();
        let config = analyses.pm_data().config();

        let skip_pass = !is_pass_manager && config.skip_passes.contains(pass.name());
        let pre_print_pass = !is_pass_manager
            && (config.print_before_all || config.print_before.contains(pass.name()));
        let post_print_pass = !is_pass_manager
            && (config.print_after_all || config.print_after.contains(pass.name()));
        let pre_verify_pass = !is_pass_manager
            && (config.verify_before_all || config.verify_before.contains(pass.name()));
        let post_verify_pass = !is_pass_manager
            && (config.verify_after_all || config.verify_after.contains(pass.name()));
        let should_time = !is_pass_manager
            && (config.time_all_passes || config.time_passes.contains(pass.name()));

        // Skip passes that are configured to be skipped, but only for non-manager passes.
        if skip_pass {
            log::debug!("Skipping pass {} on {}", pass.name(), OpDbg { op, ctx });
            return Ok(PassResult::default());
        }

        if !is_pass_manager {
            log::debug!("Running pass {} on {}", pass.name(), OpDbg { op, ctx });
        }

        if pre_print_pass {
            log::info!("IR before pass {}:\n{}", pass.name(), op.disp(ctx));
        }
        if pre_verify_pass {
            verify_operation(op, ctx).inspect_err(|e| {
                log::error!(
                    "Verification failed before pass {} on {}:\n{}",
                    pass.name(),
                    OpDbg { op, ctx },
                    e.disp(ctx)
                );
            })?;
        }
        let timer = Timer::start();
        // Run the pass and get the result.
        let result = pass.run(op, ctx, analyses);
        if should_time {
            let elapsed = timer.elapsed();
            log::info!(
                "Pass {} on {} completed in {:?}",
                pass.name(),
                OpDbg { op, ctx },
                elapsed
            );
        }
        if post_print_pass {
            log::info!("IR after pass {}:\n{}", pass.name(), op.disp(ctx));
        }
        if post_verify_pass {
            verify_operation(op, ctx).inspect_err(|e| {
                log::error!(
                    "Verification failed after pass {} on {}:\n{}",
                    pass.name(),
                    OpDbg { op, ctx },
                    e.disp(ctx)
                );
            })?;
        }
        result
    }
}

/// [PassManager] configuration.
#[derive(Default)]
pub struct PMConfig {
    /// If true, print the IR before running each pass.
    pub print_before_all: bool,
    /// If true, print the IR after running each pass.
    pub print_after_all: bool,
    /// Set of pass names for which to print the IR before execution.
    pub print_before: FxHashSet<String>,
    /// Set of pass names for which to print the IR after execution.
    pub print_after: FxHashSet<String>,
    /// If true, verify the IR before running each pass.
    pub verify_before_all: bool,
    /// If true, verify the IR after running each pass.
    pub verify_after_all: bool,
    /// Set of pass names for which to verify the IR before execution.
    pub verify_before: FxHashSet<String>,
    /// Set of pass names for which to verify the IR after execution.
    pub verify_after: FxHashSet<String>,
    /// If true, time the execution of each pass.
    pub time_all_passes: bool,
    /// Set of pass names for which to time the execution.
    pub time_passes: FxHashSet<String>,
    /// Set of pass names to skip execution.
    pub skip_passes: FxHashSet<String>,
    /// Custom configuration for extensibility.
    pub custom_config: FxHashMap<Identifier, Box<dyn core::any::Any>>,
}

/// Internal state maintained across [PassManager]s.
/// For use by [PassManager] implementations and not by passes themselves.
#[derive(Default)]
pub struct PMState {
    /// Statistics reported by passes, keyed by pass name.
    /// These statistics are printed (as requested in [PMConfig])
    /// at the end of a pass.
    pub stats: FxHashMap<&'static str, Box<dyn Printable>>,
    /// Custom state for extensibility.
    pub custom_state: FxHashMap<Identifier, Box<dyn core::any::Any>>,
}

/// Common data across [PassManager]s stored in [AnalysisManager].
#[derive(Default)]
pub struct PMData {
    /// Configuration for any [PassManager].
    config: PMConfig,
    /// Internal state across any [PassManager].
    state: PMState,
}

impl PMData {
    /// Get a reference to the [PMConfig].
    pub fn config(&self) -> &PMConfig {
        &self.config
    }

    /// Set [PMConfig]
    pub fn set_config(&mut self, config: PMConfig) {
        self.config = config;
    }

    /// Get a reference to the internal state.
    pub fn state(&self) -> &PMState {
        &self.state
    }

    /// Get a mutable reference to the internal state.
    pub fn state_mut(&mut self) -> &mut PMState {
        &mut self.state
    }
}

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
    /// Common data across [PassManager]s.
    pub pm_data: PMData,
    /// Cached analyses keyed by (TypeId of the analysis, Operation).
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

    /// Set [PMConfig]
    pub fn set_config(&mut self, config: PMConfig) {
        self.pm_data.set_config(config);
    }

    /// Get a reference to pass manager related data
    pub fn pm_data(&self) -> &PMData {
        &self.pm_data
    }

    /// Get a mutable reference to pass manager related data
    pub fn pm_data_mut(&mut self) -> &mut PMData {
        &mut self.pm_data
    }
}
