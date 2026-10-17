use crate::utils::{def_path_is, is_op};
use clippy_utils::diagnostics::span_lint_and_then;
use clippy_utils::sym;
use rustc_hir::def::Res;
use rustc_hir::def_id::DefId;
use rustc_hir::{Expr, ExprKind, QPath};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty;
use rustc_session::{declare_lint, declare_lint_pass};

declare_lint! {
    /// ### Deny explicit construction of `Op` values
    ///
    /// The lint reports a struct expression such as `MyOp { op }` for a type that implements
    /// `Op`. While a constructor of the `Op` must use `Operation::new_op`, others must use
    /// `Operation::get_op` instead, which checks that the operation is indeed a `MyOp`.
    ///
    /// ### Code reported by the lint
    ///
    /// ```rust,ignore
    /// let my_op = MyOp { op };
    /// ```
    ///
    /// ### Corrected code
    ///
    /// ```rust,ignore
    /// let my_op = Operation::get_op::<MyOp>(op, ctx).expect("op must be a MyOp");
    /// ```
    pub EXPLICIT_OP_CONSTRUCTION,
    Deny,
    "an `Op` value is constructed with a struct expression"
}

declare_lint! {
    /// ### Warn against `Self { op }` in `Op` constructors
    ///
    /// The lint reports a struct expression `Self { op }` for a type that implements `Op`.
    /// Use `Operation::new_op` instead, which creates the operation and the `Op` together.
    ///
    /// ### Code reported by the lint
    ///
    /// ```rust,ignore
    /// let op = Operation::new(ctx, Self::get_concrete_op_info(), results, operands, succs, 0);
    /// Self { op }
    /// ```
    ///
    /// ### Corrected code
    ///
    /// ```rust,ignore
    /// Operation::new_op::<Self>(ctx, results, operands, succs, 0)
    /// ```
    pub SELF_OP_CONSTRUCTION,
    Warn,
    "an `Op` constructor uses `Self { op }` instead of `Operation::new_op`"
}

/// Help text for a constructor of an `Op`.
const NEW_OP_HELP: &str = "in a constructor of the `Op`, use `Operation::new_op::<Self>(..)`, \
                           which creates the operation and the `Op` together";

declare_lint! {
    /// ### Warn against `Operation::new_op` outside the constructors of the `Op`
    ///
    /// The lint reports a call of `Operation::new_op::<MyOp>` outside an `impl` of `MyOp`.
    /// Use a constructor of `MyOp` instead, such as `MyOp::new`, which creates the operation
    /// completely.
    ///
    /// ### Code reported by the lint
    ///
    /// ```rust,ignore
    /// let my_op = Operation::new_op::<MyOp>(ctx, results, operands, succs, 0);
    /// ```
    ///
    /// ### Corrected code
    ///
    /// ```rust,ignore
    /// let my_op = MyOp::new(ctx, operands);
    /// ```
    pub NEW_OP_OUTSIDE_CONSTRUCTOR,
    Warn,
    "`Operation::new_op` creates an `Op` outside a constructor of the `Op`"
}

declare_lint_pass!(ExplicitOpConstruction => [
    EXPLICIT_OP_CONSTRUCTION,
    SELF_OP_CONSTRUCTION,
    NEW_OP_OUTSIDE_CONSTRUCTOR
]);

impl<'tcx> LateLintPass<'tcx> for ExplicitOpConstruction {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        if let ExprKind::Call(func, _) = expr.kind {
            check_new_op_call(cx, expr, func);
            return;
        }
        let ExprKind::Struct(qpath, ..) = expr.kind else {
            return;
        };
        if expr.span.from_expansion() {
            return;
        }
        let uses_self = matches!(
            qpath,
            QPath::Resolved(None, path) if matches!(path.res, Res::SelfTyAlias { .. })
        );
        let op_ty = cx.typeck_results().expr_ty(expr);
        let ty::Adt(adt, _) = op_ty.kind() else {
            return;
        };
        if !is_op(cx, op_ty) {
            return;
        }
        if uses_self {
            span_lint_and_then(
                cx,
                SELF_OP_CONSTRUCTION,
                expr.span,
                "construction of an `Op` with `Self { op }`",
                |diag| {
                    diag.help(NEW_OP_HELP);
                },
            );
            return;
        }
        let name = cx.tcx.item_name(adt.did());
        span_lint_and_then(
            cx,
            EXPLICIT_OP_CONSTRUCTION,
            expr.span,
            format!("explicit construction of the `Op` `{name}`"),
            |diag| {
                diag.help(format!(
                    "use `Operation::get_op::<{name}>(op, ctx)`, which checks that the \
                     operation is a `{name}`"
                ));
            },
        );
    }
}

/// Report a call of `Operation::new_op::<T>` for a concrete `Op` `T`, outside an `impl` of `T`.
fn check_new_op_call<'tcx>(cx: &LateContext<'tcx>, call: &'tcx Expr<'tcx>, func: &'tcx Expr<'tcx>) {
    let ExprKind::Path(ref qpath) = func.kind else {
        return;
    };
    let Some(did) = cx.qpath_res(qpath, func.hir_id).opt_def_id() else {
        return;
    };
    if call.span.from_expansion() || !def_path_is(cx, did, &["pliron", "operation", "new_op"]) {
        return;
    }
    // A generic `T` (also `Self` in a trait) is not a concrete `Op`.
    let ty::Adt(adt, _) = cx.typeck_results().node_args(func.hir_id).type_at(0).kind() else {
        return;
    };
    if in_impl_of(cx, call, adt.did()) {
        return;
    }
    let name = cx.tcx.item_name(adt.did());
    let has_new = cx.tcx.inherent_impls(adt.did()).iter().any(|&impl_id| {
        cx.tcx
            .associated_items(impl_id)
            .filter_by_name_unhygienic(sym::new)
            .next()
            .is_some()
    });
    span_lint_and_then(
        cx,
        NEW_OP_OUTSIDE_CONSTRUCTOR,
        call.span,
        format!("`Operation::new_op` creates a `{name}` outside a constructor of `{name}`"),
        |diag| {
            if has_new {
                diag.help(format!("use `{name}::new`"));
            } else {
                diag.help(format!("use a constructor of `{name}`"));
            }
        },
    );
}

/// Return true if `expr` is in a method of an `impl` of the type `adt_did`.
fn in_impl_of(cx: &LateContext<'_>, expr: &Expr<'_>, adt_did: DefId) -> bool {
    cx.tcx
        .impl_of_assoc(cx.tcx.hir_get_parent_item(expr.hir_id).to_def_id())
        .is_some_and(|impl_id| {
            matches!(
                cx.tcx.type_of(impl_id).instantiate_identity().skip_norm_wip().kind(),
                ty::Adt(impl_adt, _) if impl_adt.did() == adt_did
            )
        })
}
