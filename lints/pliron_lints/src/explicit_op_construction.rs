use clippy_utils::diagnostics::span_lint_and_then;
use clippy_utils::paths::{PathNS, lookup_path_str};
use clippy_utils::ty::implements_trait;
use rustc_hir::def::Res;
use rustc_hir::{Expr, ExprKind, QPath};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty;
use rustc_session::{declare_lint, declare_lint_pass};

declare_lint! {
    /// ### Deny explicit construction of `Op` values
    ///
    /// The lint reports a struct expression such as `MyOp { op }` for a type that implements
    /// `Op`. A constructor of the `Op` must use `Operation::new_op`, which creates the operation
    /// and the `Op` together. Other code must use `Operation::get_op`, which checks that the
    /// operation is a `MyOp`.
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

declare_lint_pass!(ExplicitOpConstruction => [EXPLICIT_OP_CONSTRUCTION, SELF_OP_CONSTRUCTION]);

impl<'tcx> LateLintPass<'tcx> for ExplicitOpConstruction {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
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
        let is_op = lookup_path_str(cx.tcx, PathNS::Type, "pliron::op::Op")
            .into_iter()
            .any(|op_trait| implements_trait(cx, op_ty, op_trait, &[]));
        if !is_op {
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
        // A method in an `impl` of the same `Op` is a constructor of that `Op`.
        let in_own_impl = cx
            .tcx
            .impl_of_assoc(cx.tcx.hir_get_parent_item(expr.hir_id).to_def_id())
            .is_some_and(|impl_id| {
                matches!(
                    cx.tcx.type_of(impl_id).instantiate_identity().skip_norm_wip().kind(),
                    ty::Adt(impl_adt, _) if impl_adt.did() == adt.did()
                )
            });
        span_lint_and_then(
            cx,
            EXPLICIT_OP_CONSTRUCTION,
            expr.span,
            format!("explicit construction of the `Op` `{name}`"),
            |diag| {
                if in_own_impl {
                    diag.help(NEW_OP_HELP);
                } else {
                    diag.help(format!(
                        "use `Operation::get_op::<{name}>(op, ctx)`, which checks that the \
                         operation is a `{name}`"
                    ));
                }
            },
        );
    }
}
