use crate::utils::{PRINTABLE_TRAIT, binds_name, pliron_trait_ids};
use clippy_utils::diagnostics::span_lint_and_then;
use clippy_utils::source::snippet_with_applicability;
use rustc_errors::Applicability;
use rustc_hir::def::DefKind;
use rustc_hir::{Expr, ExprKind, PatKind};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::Ident;

declare_lint! {
    /// ### Warn against `disp` in an `impl Printable`
    ///
    /// The lint reports a call of `disp` in the `fmt` method of an `impl Printable`.
    /// `disp` prints with a default `State`, so the state of `fmt` (for example the
    /// indentation) does not reach the printed value. `print` takes the state.
    ///
    /// ### Code reported by the lint
    ///
    /// ```rust,ignore
    /// fn fmt(&self, ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    ///     write!(f, "{}", self.ty.disp(ctx))
    /// }
    /// ```
    ///
    /// ### Corrected code
    ///
    /// ```rust,ignore
    /// fn fmt(&self, ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    ///     write!(f, "{}", self.ty.print(ctx, state))
    /// }
    /// ```
    pub DISP_IN_PRINTABLE,
    Warn,
    "`disp` in an `impl Printable` does not pass the `State` of `fmt`"
}

declare_lint_pass!(DispInPrintable => [DISP_IN_PRINTABLE]);

impl<'tcx> LateLintPass<'tcx> for DispInPrintable {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        let ExprKind::MethodCall(segment, receiver, [ctx_arg], _) = expr.kind else {
            return;
        };
        if segment.ident.as_str() != "disp" || expr.span.in_external_macro(cx.sess().source_map()) {
            return;
        }
        let printable_traits = pliron_trait_ids(cx, PRINTABLE_TRAIT);
        let is_printable_disp = cx
            .typeck_results()
            .type_dependent_def_id(expr.hir_id)
            .and_then(|did| cx.tcx.trait_of_assoc(did))
            .is_some_and(|trait_id| printable_traits.contains(&trait_id));
        if !is_printable_disp {
            return;
        }
        let Some(state) = printable_fmt_state(cx, expr, &printable_traits) else {
            return;
        };
        span_lint_and_then(
            cx,
            DISP_IN_PRINTABLE,
            expr.span,
            "`disp` prints with a default `State`, not the `State` of `fmt`",
            |diag| {
                let state = match state {
                    StateParam::Named(state) => state,
                    StateParam::Unnamed => {
                        diag.help("use `print` with the `State` of `fmt`");
                        return;
                    }
                    StateParam::Shadowed(state) => {
                        diag.help(format!(
                            "use `print` with the `State` of `fmt`; the body of `fmt` binds the \
                             name `{state}` again"
                        ));
                        return;
                    }
                };
                let mut app = if state.as_str().starts_with('_') {
                    Applicability::MaybeIncorrect
                } else {
                    Applicability::MachineApplicable
                };
                let receiver = snippet_with_applicability(cx, receiver.span, "..", &mut app);
                let ctx_arg = snippet_with_applicability(cx, ctx_arg.span, "..", &mut app);
                diag.span_suggestion(
                    expr.span,
                    "use `print` with the `State` of `fmt`",
                    format!("{receiver}.print({ctx_arg}, {state})"),
                    app,
                );
            },
        );
    }
}

/// The `State` parameter of a `fmt` method.
enum StateParam {
    /// The parameter has a name that the body does not bind again.
    Named(Ident),
    /// The parameter has a name that the body binds again, so the name can refer to another
    /// variable.
    Shadowed(Ident),
    /// The parameter has no name, for example `_`.
    Unnamed,
}

/// If `expr` is in the `fmt` method of an `impl Printable`, describe the `State` parameter of
/// `fmt`.
fn printable_fmt_state<'tcx>(
    cx: &LateContext<'tcx>,
    expr: &'tcx Expr<'tcx>,
    printable_traits: &[rustc_hir::def_id::DefId],
) -> Option<StateParam> {
    let fn_id = cx.tcx.hir_get_parent_item(expr.hir_id).def_id;
    if cx.tcx.item_name(fn_id.to_def_id()).as_str() != "fmt" {
        return None;
    }
    let impl_id = cx.tcx.impl_of_assoc(fn_id.to_def_id())?;
    if !matches!(cx.tcx.def_kind(impl_id), DefKind::Impl { of_trait: true }) {
        return None;
    }
    let trait_id = cx.tcx.impl_trait_ref(impl_id).skip_binder().def_id;
    if !printable_traits.contains(&trait_id) {
        return None;
    }
    // The parameters of `fmt` are `self`, `ctx`, `state` and `f`.
    let body = cx.tcx.hir_body_owned_by(fn_id);
    let state = body.params.get(2)?;
    Some(match state.pat.kind {
        PatKind::Binding(_, _, ident, None) if binds_name(cx, body.value, ident.as_str()) => {
            StateParam::Shadowed(ident)
        }
        PatKind::Binding(_, _, ident, None) => StateParam::Named(ident),
        _ => StateParam::Unnamed,
    })
}
