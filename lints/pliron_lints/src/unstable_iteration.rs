use crate::utils::def_path_is;
use clippy_utils::diagnostics::span_lint_and_help;
use rustc_hir::def_id::DefId;
use rustc_hir::{Expr, ExprKind};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_middle::ty;
use rustc_session::{declare_lint, declare_lint_pass};

/// Definition paths of the hash tables with an iteration order that is not specified.
const UNSTABLE_TABLE_PATHS: [[&str; 5]; 2] = [
    ["pliron", "utils", "table", "htable", "HMap"],
    ["pliron", "utils", "table", "htable", "HSet"],
];

declare_lint! {
    /// ### Warn against iteration of an `HMap` or an `HSet`
    ///
    /// The lint reports a use of an `unstable_*` method of `HMap` or `HSet`.
    /// These methods iterate in an order that is not specified, so the result
    /// can change with the platform or the sequence of inserts and removals.
    /// `IMap` and `ISet` iterate in a deterministic order.
    ///
    /// If the result is order independent, the lint must be explicitly allowed at that site.
    ///
    /// ### Code reported by the lint
    ///
    /// ```rust,ignore
    /// let names: Vec<_> = map.unstable_values().collect();
    /// ```
    ///
    /// ### Corrected code
    ///
    /// ```rust,ignore
    /// // `map` is an `IMap`.
    /// let names: Vec<_> = map.values().collect();
    /// ```
    pub UNSTABLE_ITERATION,
    Warn,
    "iteration of an `HMap` or an `HSet` in an order that is not specified"
}

declare_lint_pass!(UnstableIteration => [UNSTABLE_ITERATION]);

impl<'tcx> LateLintPass<'tcx> for UnstableIteration {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        let did = match expr.kind {
            ExprKind::MethodCall(..) => cx.typeck_results().type_dependent_def_id(expr.hir_id),
            ExprKind::Path(ref qpath) => cx.qpath_res(qpath, expr.hir_id).opt_def_id(),
            _ => None,
        };
        let Some(did) = did else {
            return;
        };
        if expr.span.in_external_macro(cx.sess().source_map()) {
            return;
        }
        let Some(table) = unstable_table_method(cx, did) else {
            return;
        };
        let method = cx.tcx.item_name(did);
        span_lint_and_help(
            cx,
            UNSTABLE_ITERATION,
            expr.span,
            format!("`{table}::{method}` iterates in an order that is not specified"),
            None,
            "use `IMap` or `ISet` for a deterministic order; if the result is order independent, \
             the lint must be explicitly allowed at this site",
        );
    }
}

/// If `did` is an `unstable_*` method of `HMap` or `HSet`, return the name of the table.
fn unstable_table_method(cx: &LateContext<'_>, did: DefId) -> Option<&'static str> {
    if !cx.tcx.item_name(did).as_str().starts_with("unstable_") {
        return None;
    }
    let impl_id = cx.tcx.impl_of_assoc(did)?;
    let ty::Adt(adt, _) = cx.tcx.type_of(impl_id).skip_binder().kind() else {
        return None;
    };
    UNSTABLE_TABLE_PATHS
        .iter()
        .find(|path| def_path_is(cx, adt.did(), path.as_slice()))
        .map(|path| path[4])
}
