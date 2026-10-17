use crate::utils::{
    Guard, binds_name, first_risky_context_use, pat_binds, scrutinee_guard_temporaries,
};
use clippy_utils::diagnostics::span_lint_and_then;
use clippy_utils::higher;
use clippy_utils::source::{indent_of, snippet_with_applicability, walk_span_to_context};
use clippy_utils::visitors::for_each_expr;
use rustc_data_structures::fx::FxHashSet;
use rustc_errors::{Applicability, Diag};
use rustc_hir::def::Res;
use rustc_hir::{Expr, ExprKind, MatchSource, Node, QPath};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::edition::Edition;
use rustc_span::{Span, Symbol, SyntaxContext};
use std::ops::ControlFlow;

declare_lint! {
    /// ### Warn against `Ref` temporaries in loop and `match` headers
    ///
    /// The lint applies to the iterable of a `for` loop, and to the scrutinee of a `match`,
    /// `if let` or `while let`. It reports a `Ref` or `RefMut` temporary there only when
    /// the body uses a `Context`.
    ///
    /// ### Risk of a run-time borrow panic
    ///
    /// A temporary in these positions is not dropped at the end of the expression.
    /// It lives until the end of the full loop or `match`. While the body runs, the
    /// `RefCell` stays borrowed. If the body (or code that it calls with the `Context`)
    /// calls `deref_mut` on the same object, the program panics at run time.
    ///
    /// ### Code reported by the lint
    ///
    /// ```rust,ignore
    /// for op in block.deref(ctx).iter(ctx) {
    ///     process_op(op, ctx);
    /// }
    /// ```
    ///
    /// ### Corrected code
    ///
    /// ```rust,ignore
    /// for op in block.iter(ctx) {
    ///     process_op(op, ctx);
    /// }
    /// ```
    ///
    /// `Ptr::iter` borrows the block only during the call. For other values that do not
    /// borrow the `Ref` (for example `succs(ctx)` or `get_parent_op(ctx)`), move the value
    /// into a `let` statement before the loop or `match`. The lint suggests this change.
    /// If the value borrows the `Ref` (for example `results()` or `arguments()`), collect the
    /// values into a `Vec` first.
    ///
    /// The lint does not report `operands()`, `successors()` and `regions()` of an operation.
    /// The uses of their items are kept in other objects. The uses of the items of `results()`
    /// and `arguments()` are kept in the borrowed object, so a new use of such an item panics.
    pub IR_REF_IN_SCRUTINEE,
    Warn,
    "a `Ref` temporary stays borrowed during a loop or `match` body that uses a `Context`"
}

#[derive(Default)]
pub struct IrRefInScrutinee {
    /// Spans of the temporaries reported by the lint.
    reported: FxHashSet<Span>,
}

impl_lint_pass!(IrRefInScrutinee => [IR_REF_IN_SCRUTINEE]);

impl<'tcx> LateLintPass<'tcx> for IrRefInScrutinee {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        if expr.span.in_external_macro(cx.sess().source_map()) {
            return;
        }

        let (construct, scrutinee, bodies): (&str, &'tcx Expr<'tcx>, Vec<&'tcx Expr<'tcx>>) =
            if let Some(for_loop) = higher::ForLoop::hir(expr) {
                ("for", for_loop.arg, vec![for_loop.body])
            } else if let Some(while_let) = higher::WhileLet::hir(expr) {
                ("while let", while_let.let_expr, vec![while_let.if_then])
            } else if let Some(if_let) = higher::IfLet::hir(cx, expr) {
                let mut bodies = vec![if_let.if_then];
                // Before edition 2024, the temporaries also live during the `else` block.
                if expr.span.edition() < Edition::Edition2024
                    && let Some(if_else) = if_let.if_else
                {
                    bodies.push(if_else);
                }
                ("if let", if_let.let_expr, bodies)
            } else if let ExprKind::Match(scrutinee, arms, MatchSource::Normal) = expr.kind {
                let bodies = arms
                    .iter()
                    .flat_map(|arm| arm.guard.into_iter().chain(core::iter::once(arm.body)))
                    .collect();
                ("match", scrutinee, bodies)
            } else {
                return;
            };

        for (temporary, guard) in scrutinee_guard_temporaries(cx, scrutinee) {
            if is_component_iterator(cx, temporary, &guard) {
                continue;
            }
            let Some(context_use) = bodies
                .iter()
                .find_map(|body| first_risky_context_use(cx, *body, &guard))
            else {
                continue;
            };
            if !self.reported.insert(temporary.span) {
                continue;
            }
            span_lint_and_then(
                cx,
                IR_REF_IN_SCRUTINEE,
                temporary.span,
                format!(
                    "this `{}` temporary stays alive until the end of the `{construct}`",
                    guard.name
                ),
                |diag| {
                    diag.span_note(context_use, guard.context_use_note());
                    diag.note(guard.panic_note());
                    add_fix(cx, diag, expr, construct, scrutinee, temporary, &guard);
                },
            );
        }
    }
}

/// Add the help or suggestion that fixes the lint to `diag`.
fn add_fix<'tcx>(
    cx: &LateContext<'tcx>,
    diag: &mut Diag<'_, ()>,
    construct_expr: &'tcx Expr<'tcx>,
    construct: &str,
    scrutinee: &'tcx Expr<'tcx>,
    temporary: &'tcx Expr<'tcx>,
    guard: &Guard,
) {
    let kind = guard.kind;
    let Some((call, borrows)) = call_on_guard(cx, temporary) else {
        diag.help(format!(
            "bind the value to a `let` before the `{construct}`; \
             if the value borrows the `{kind}`, collect it into a `Vec`"
        ));
        return;
    };
    // `ptr.deref(ctx).method(..)` becomes `ptr.method(ctx)`, for the methods that `Ptr` also has.
    if let Some((method, ptr, ctx_arg)) = ptr_method_on_deref(cx, call, temporary, guard) {
        let mut app = Applicability::MachineApplicable;
        let ptr = snippet_with_applicability(cx, ptr.span, "..", &mut app);
        let ctx_arg = snippet_with_applicability(cx, ctx_arg.span, "..", &mut app);
        diag.span_suggestion(
            call.span,
            format!(
                "use `Ptr::{method}`, which does not keep the `{}` borrowed",
                guard.entity
            ),
            format!("{ptr}.{method}({ctx_arg})"),
            app,
        );
        return;
    }

    if borrows {
        diag.help(format!(
            "the value borrows the `{kind}`: collect the values that you need into a `Vec` \
             before the `{construct}`"
        ));
        return;
    }

    // A `while let` evaluates the scrutinee in each iteration, so a `let` before the loop
    // changes the behavior.
    let statement_span = walk_span_to_context(construct_expr.span, SyntaxContext::root());
    if construct == "while let" || !is_statement(cx, construct_expr) || statement_span.is_none() {
        diag.help(format!(
            "the value does not borrow the `{kind}`: bind it to a `let` that is dropped \
             before the body runs"
        ));
        return;
    }
    let statement_span = statement_span.unwrap();
    let name = binding_name(call, construct);
    let mut app = if name_is_used(cx, &name) || call.span.lo() != scrutinee.span.lo() {
        Applicability::MaybeIncorrect
    } else {
        Applicability::MachineApplicable
    };
    let value = snippet_with_applicability(cx, call.span, "..", &mut app);
    let indent = " ".repeat(indent_of(cx, statement_span).unwrap_or(0));
    diag.multipart_suggestion(
        format!(
            "the value does not borrow the `{kind}`: move it into a `let` before the \
             `{construct}`, so that the `{kind}` is dropped at the end of that statement"
        ),
        vec![
            (
                statement_span.shrink_to_lo(),
                format!("let {name} = {value};\n{indent}"),
            ),
            (call.span, name),
        ],
        app,
    );
}

/// If `temporary` is the receiver of a method call, return that call, and whether the
/// result of the call can borrow the receiver (its return type uses the lifetime of `self`).
fn call_on_guard<'tcx>(
    cx: &LateContext<'tcx>,
    temporary: &'tcx Expr<'tcx>,
) -> Option<(&'tcx Expr<'tcx>, bool)> {
    let Node::Expr(call) = cx.tcx.parent_hir_node(temporary.hir_id) else {
        return None;
    };
    let ExprKind::MethodCall(_, receiver, _, _) = call.kind else {
        return None;
    };
    if receiver.hir_id != temporary.hir_id {
        return None;
    }
    let did = cx.typeck_results().type_dependent_def_id(call.hir_id)?;
    let sig = cx.tcx.fn_sig(did).instantiate_identity().skip_binder();
    let self_ty = *sig.inputs().first()?;
    let rustc_middle::ty::Ref(self_region, _, _) = self_ty.kind() else {
        return Some((call, true));
    };
    let borrows = sig
        .output()
        .walk()
        .any(|arg| arg.as_region() == Some(*self_region));
    Some((call, borrows))
}

/// Iterators over the parts of an IR entity, which the lint does not report.
const COMPONENT_ITERATORS: [(&str, &str); 3] = [
    ("operands", "Operation"),
    ("successors", "Operation"),
    ("regions", "Operation"),
];

/// Return true if `temporary` is a `Ref` and the receiver of a pliron method
/// in [COMPONENT_ITERATORS].
fn is_component_iterator<'tcx>(
    cx: &LateContext<'tcx>,
    temporary: &'tcx Expr<'tcx>,
    guard: &Guard,
) -> bool {
    if guard.kind != "Ref" {
        return false;
    }
    let Node::Expr(call) = cx.tcx.parent_hir_node(temporary.hir_id) else {
        return false;
    };
    let ExprKind::MethodCall(segment, receiver, [], _) = call.kind else {
        return false;
    };
    receiver.hir_id == temporary.hir_id
        && COMPONENT_ITERATORS
            .iter()
            .any(|(method, entity)| segment.ident.as_str() == *method && guard.entity == *entity)
        && cx
            .typeck_results()
            .type_dependent_def_id(call.hir_id)
            .is_some_and(|did| cx.tcx.crate_name(did.krate).as_str() == "pliron")
}

/// The methods that `Ptr` also has: the method name, the IR entity of the receiver, and
/// whether the method takes the `Context` itself. `iter` is `ContainsLinkedList::iter`.
const PTR_METHODS: [(&str, &str, bool); 2] =
    [("iter", "BasicBlock", true), ("iter", "Region", true)];

/// If `call` is `ptr.deref(ctx).method(..)` for a method in [PTR_METHODS], return the method
/// name, `ptr` and the `ctx` argument for `Ptr::method`.
fn ptr_method_on_deref<'tcx>(
    cx: &LateContext<'tcx>,
    call: &'tcx Expr<'tcx>,
    temporary: &'tcx Expr<'tcx>,
    guard: &Guard,
) -> Option<(&'static str, &'tcx Expr<'tcx>, &'tcx Expr<'tcx>)> {
    let ExprKind::MethodCall(segment, _, call_args, _) = call.kind else {
        return None;
    };
    let ExprKind::MethodCall(deref_segment, ptr, [deref_ctx_arg], _) = temporary.kind else {
        return None;
    };
    let in_pliron = |hir_id| {
        cx.typeck_results()
            .type_dependent_def_id(hir_id)
            .is_some_and(|did| cx.tcx.crate_name(did.krate).as_str() == "pliron")
    };
    if deref_segment.ident.as_str() != "deref"
        || !in_pliron(temporary.hir_id)
        || !in_pliron(call.hir_id)
        || call.span.from_expansion()
    {
        return None;
    }
    let &(method, _, takes_context) = PTR_METHODS
        .iter()
        .find(|(method, entity, _)| segment.ident.as_str() == *method && guard.entity == *entity)?;
    let ctx_arg = match (takes_context, call_args) {
        (true, [ctx_arg]) => ctx_arg,
        (false, []) => deref_ctx_arg,
        _ => return None,
    };
    Some((method, ptr, ctx_arg))
}

/// Return true if `expr` is a statement, or the tail expression of a block.
fn is_statement<'tcx>(cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) -> bool {
    match cx.tcx.parent_hir_node(expr.hir_id) {
        Node::Stmt(_) => true,
        Node::Block(block) => block.expr.is_some_and(|tail| tail.hir_id == expr.hir_id),
        _ => false,
    }
}

/// Name of the new `let` binding: the method name of `call` without a `get_` prefix
/// (`items` for a `for` loop over `iter`). If that name is a keyword, the name is `value`.
fn binding_name(call: &Expr<'_>, construct: &str) -> String {
    let ExprKind::MethodCall(segment, ..) = call.kind else {
        return "value".to_string();
    };
    let name = segment.ident.as_str();
    let name = name.strip_prefix("get_").unwrap_or(name);
    if name == "iter" && construct == "for" {
        "items".to_string()
    } else if Symbol::intern(name).is_reserved(|| Edition::Edition2024) {
        "value".to_string()
    } else {
        name.to_string()
    }
}

/// Return true if the enclosing body uses the identifier `name` as a local or a binding.
fn name_is_used(cx: &LateContext<'_>, name: &str) -> bool {
    let Some(body_id) = cx.enclosing_body else {
        return true;
    };
    let body = cx.tcx.hir_body(body_id);
    let in_params = body.params.iter().any(|param| pat_binds(param.pat, name));
    let uses_local = for_each_expr(cx.tcx, body.value, |e| match e.kind {
        ExprKind::Path(QPath::Resolved(None, path))
            if matches!(path.res, Res::Local(_))
                && path
                    .segments
                    .last()
                    .is_some_and(|seg| seg.ident.as_str() == name) =>
        {
            ControlFlow::Break(())
        }
        _ => ControlFlow::Continue(()),
    })
    .is_some();
    in_params || uses_local || binds_name(cx, body.value, name)
}
