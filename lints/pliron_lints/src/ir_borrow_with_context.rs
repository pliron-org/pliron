use crate::utils::{ir_entity_borrow, is_context};
use clippy_utils::diagnostics::span_lint_and_then;
use rustc_hir::def::DefKind;
use rustc_hir::intravisit::FnKind;
use rustc_hir::{Body, FnDecl};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::Span;
use rustc_span::def_id::LocalDefId;

declare_lint! {
    /// ### Warn against IR object borrows in functions that take a `Context`
    ///
    /// The lint reports a parameter of type `&Operation`, `&BasicBlock` or `&Region`
    /// (shared or mutable), or a `Ref` or `RefMut` of one of these types, when the same
    /// function also takes a `Context`.
    ///
    /// ### Risk of a run-time borrow panic
    ///
    /// The `RefCell` of the IR object stays borrowed for the full call.
    /// Because `Context` is available, the function can reach other code
    /// that calls `deref_mut` on the same object, leading to an interior
    /// mutability panic.
    ///
    /// ### Code reported by the lint
    ///
    /// ```rust,ignore
    /// fn visit(op: &Operation, ctx: &Context) { /* ... */ }
    /// ```
    ///
    /// ### Corrected code
    ///
    /// ```rust,ignore
    /// fn visit(op: Ptr<Operation>, ctx: &Context) { /* ... */ }
    /// ```
    pub IR_BORROW_WITH_CONTEXT,
    Warn,
    "a function takes a borrow of an IR object and also a `Context`"
}

declare_lint_pass!(IrBorrowWithContext => [IR_BORROW_WITH_CONTEXT]);

impl<'tcx> LateLintPass<'tcx> for IrBorrowWithContext {
    fn check_fn(
        &mut self,
        cx: &LateContext<'tcx>,
        kind: FnKind<'tcx>,
        decl: &'tcx FnDecl<'tcx>,
        _body: &'tcx Body<'tcx>,
        span: Span,
        def_id: LocalDefId,
    ) {
        if matches!(kind, FnKind::Closure) || span.in_external_macro(cx.sess().source_map()) {
            return;
        }

        let parent = cx.tcx.local_parent(def_id);
        if matches!(
            cx.tcx.def_kind(parent),
            DefKind::Fn | DefKind::AssocFn | DefKind::Closure
        ) {
            // A function defined inside another function is called only from that function.
            // This gives the whole picture in one place, so a real bug (panic) is unlikely.
            return;
        }
        let sig = cx.tcx.fn_sig(def_id).instantiate_identity().skip_binder();
        let inputs = sig.inputs();
        let Some(context_span) = inputs
            .iter()
            .zip(decl.inputs.iter())
            .find_map(|(ty, hir_ty)| is_context(cx, *ty).then_some(hir_ty.span))
        else {
            return;
        };
        let has_receiver = cx
            .tcx
            .opt_associated_item(def_id.to_def_id())
            .is_some_and(|item| item.is_method());
        let params = inputs
            .iter()
            .zip(decl.inputs.iter())
            .skip(usize::from(has_receiver));
        for (ty, hir_ty) in params {
            if let Some(name) = ir_entity_borrow(cx, *ty) {
                span_lint_and_then(
                    cx,
                    IR_BORROW_WITH_CONTEXT,
                    hir_ty.span,
                    format!(
                        "this `{name}` stays borrowed for the full call of a function \
                         that also takes a `Context`"
                    ),
                    |diag| {
                        diag.span_note(
                            context_span,
                            format!(
                                "a second borrow of the `{name}` is possible through this \
                                 `Context`"
                            ),
                        );
                        diag.note(format!(
                            "a `deref_mut` of the `{name}` during the call panics at run time"
                        ));
                        diag.help(format!(
                            "take a `Ptr<{name}>` instead, and call `deref` only around each read"
                        ));
                    },
                );
            }
        }
    }
}
