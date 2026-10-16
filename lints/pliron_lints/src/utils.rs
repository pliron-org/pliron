//! Type and expression helpers shared by the lints.

use clippy_utils::paths::{PathNS, lookup_path_str};
use clippy_utils::sym;
use clippy_utils::ty::implements_trait;
use clippy_utils::visitors::{Descend, Visitable, for_each_expr};
use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_hir::{Expr, ExprKind, Node, UnOp};
use rustc_lint::LateContext;
use rustc_middle::ty::adjustment::{Adjust, DerefAdjustKind};
use rustc_middle::ty::print::with_forced_trimmed_paths;
use rustc_middle::ty::{self, Ty};
use rustc_span::Span;
use std::ops::ControlFlow;

/// Definition path of `pliron::context::Context`.
const CONTEXT_PATH: [&str; 3] = ["pliron", "context", "Context"];

/// Definition paths of the IR entities.
const IR_ENTITY_PATHS: [[&str; 3]; 3] = [
    ["pliron", "operation", "Operation"],
    ["pliron", "basic_block", "BasicBlock"],
    ["pliron", "region", "Region"],
];

/// Return true if the definition path of `did` (including the crate name) is `path`.
fn def_path_is(cx: &LateContext<'_>, did: DefId, path: &[&str]) -> bool {
    let tcx = cx.tcx;
    let segments: Vec<_> = core::iter::once(tcx.crate_name(did.krate))
        .chain(
            tcx.def_path(did)
                .data
                .iter()
                .filter_map(|d| d.data.get_opt_name()),
        )
        .collect();
    segments.len() == path.len()
        && segments
            .iter()
            .zip(path.iter())
            .all(|(segment, expected)| segment.as_str() == *expected)
}

/// Return true if `ty`, after all references are removed, is `pliron::context::Context`.
pub fn is_context<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> bool {
    match ty.peel_refs().kind() {
        ty::Adt(adt, _) => def_path_is(cx, adt.did(), &CONTEXT_PATH),
        _ => false,
    }
}

/// If `ty` is an IR entity (`Operation`, `BasicBlock` or `Region`), return its name.
pub fn ir_entity_name<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> Option<&'static str> {
    let ty::Adt(adt, _) = ty.kind() else {
        return None;
    };
    IR_ENTITY_PATHS
        .iter()
        .find(|path| def_path_is(cx, adt.did(), path.as_slice()))
        .map(|path| path[2])
}

/// If `ty` is `core::cell::Ref` or `core::cell::RefMut`, return its name.
pub fn refcell_guard_name<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> Option<&'static str> {
    let ty::Adt(adt, _) = ty.kind() else {
        return None;
    };
    if cx.tcx.is_diagnostic_item(sym::RefCellRef, adt.did()) {
        Some("Ref")
    } else if cx.tcx.is_diagnostic_item(sym::RefCellRefMut, adt.did()) {
        Some("RefMut")
    } else {
        None
    }
}

/// A `Ref` or `RefMut` of an IR entity, described for diagnostics.
pub struct Guard {
    /// The guard type: `Ref` or `RefMut`.
    pub kind: &'static str,
    /// The guard type with its target.
    pub name: String,
    /// The IR entity borrowed by the guard.
    pub entity: &'static str,
}

impl Guard {
    /// Note text for the span of a use of a `Context` while the guard is alive.
    pub fn context_use_note(&self) -> String {
        format!(
            "the `Context` is used here while the `{}` is still borrowed",
            self.entity
        )
    }

    /// Note text for the run-time effect of a borrow during the use of the `Context`.
    pub fn panic_note(&self) -> String {
        // A `RefMut` also makes a shared borrow panic.
        let calls = if self.kind == "RefMut" {
            "a `deref` or `deref_mut`"
        } else {
            "a `deref_mut`"
        };
        format!(
            "{calls} of the `{}` during this call panics at run time",
            self.entity
        )
    }
}

/// If `e` makes a `Ref` or `RefMut` of an IR entity, return its description.
/// A `Ref` whose IR entity is not known (for example a `Ref` of a type) gives `None`.
pub fn refcell_guard<'tcx>(cx: &LateContext<'tcx>, e: &'tcx Expr<'tcx>) -> Option<Guard> {
    let ty = cx.typeck_results().expr_ty(e);
    let kind = refcell_guard_name(cx, ty)?;
    let ty::Adt(_, args) = ty.kind() else {
        return None;
    };
    let target_name = with_forced_trimmed_paths!(args.type_at(1).to_string());
    Some(Guard {
        kind,
        name: format!("{kind}<{target_name}>"),
        entity: guard_entity(cx, e)?,
    })
}

/// Find the IR entity borrowed by the guard made by `e`. The type of `e` is a guard, or an
/// `Option` or `Result` of a guard. The entity is known in these cases:
///
/// * The guard target is an IR entity, for example `Ref<Operation>`.
/// * `e` is a call of `Ref::map`, `Ref::filter_map`, `RefMut::map` or `RefMut::filter_map`:
///   the entity of the original guard (the first argument).
/// * `e` is a method of an `Op` that takes a `Context`, for example an attribute getter:
///   an `Op` holds only its `Operation`, so the guard borrows that `Operation`.
/// * `e` is a method on a guard, or on an `Option` or `Result` of a guard (for example
///   `unwrap` or `ok`): the entity of the receiver.
fn guard_entity<'tcx>(cx: &LateContext<'tcx>, e: &'tcx Expr<'tcx>) -> Option<&'static str> {
    let guard_ty = guard_in(cx, cx.typeck_results().expr_ty(e))?;
    let ty::Adt(_, args) = guard_ty.kind() else {
        return None;
    };
    if let Some(entity) = ir_entity_name(cx, args.type_at(1)) {
        return Some(entity);
    }
    match e.kind {
        ExprKind::Call(func, [original, ..]) if is_guard_map(cx, func) => {
            guard_entity(cx, original)
        }
        ExprKind::MethodCall(_, receiver, args, _) => {
            let receiver_ty = cx.typeck_results().expr_ty(receiver).peel_refs();
            let takes_context = core::iter::once(receiver)
                .chain(args.iter())
                .any(|arg| is_context(cx, cx.typeck_results().expr_ty(arg)));
            if takes_context && is_op(cx, receiver_ty) {
                Some("Operation")
            } else if guard_in(cx, receiver_ty).is_some() {
                guard_entity(cx, receiver)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// If `ty` is a guard, or an `Option` or `Result` of a guard, return the guard type.
fn guard_in<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    if refcell_guard_name(cx, ty).is_some() {
        return Some(ty);
    }
    let ty::Adt(adt, args) = ty.kind() else {
        return None;
    };
    let is_wrapper = cx.tcx.is_diagnostic_item(sym::Option, adt.did())
        || cx.tcx.is_diagnostic_item(sym::Result, adt.did());
    let inner = args.types().next()?;
    (is_wrapper && refcell_guard_name(cx, inner).is_some()).then_some(inner)
}

/// Return true if `func` is `Ref::map`, `Ref::filter_map`, `RefMut::map` or `RefMut::filter_map`.
fn is_guard_map<'tcx>(cx: &LateContext<'tcx>, func: &'tcx Expr<'tcx>) -> bool {
    let ExprKind::Path(ref qpath) = func.kind else {
        return false;
    };
    let Some(did) = cx.qpath_res(qpath, func.hir_id).opt_def_id() else {
        return false;
    };
    if !matches!(cx.tcx.item_name(did).as_str(), "map" | "filter_map") {
        return false;
    }
    let impl_did = cx.tcx.parent(did);
    matches!(cx.tcx.def_kind(impl_did), DefKind::Impl { .. })
        && refcell_guard_name(
            cx,
            cx.tcx
                .type_of(impl_did)
                .instantiate_identity()
                .skip_norm_wip(),
        )
        .is_some()
}

/// Return true if `ty` implements the `pliron::op::Op` trait.
fn is_op<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> bool {
    lookup_path_str(cx.tcx, PathNS::Type, "pliron::op::Op")
        .into_iter()
        .any(|op_trait| implements_trait(cx, ty, op_trait, &[]))
}

/// If `ty` borrows an IR entity, return the entity name.
/// That is: `&T`, `&mut T`, `Ref<T>` or `RefMut<T>` (also behind references),
/// where `T` is an IR entity.
pub fn ir_entity_borrow<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> Option<&'static str> {
    let peeled = ty.peel_refs();
    if peeled != ty
        && let Some(name) = ir_entity_name(cx, peeled)
    {
        return Some(name);
    }
    if let ty::Adt(_, args) = peeled.kind()
        && refcell_guard_name(cx, peeled).is_some()
    {
        return ir_entity_name(cx, args.type_at(1));
    }
    None
}

/// Return the span of the first expression in `node` that is a `Context` (or a reference to it).
/// The search includes closure bodies.
pub fn first_context_use<'tcx>(cx: &LateContext<'tcx>, node: impl Visitable<'tcx>) -> Option<Span> {
    first_context_use_where(cx, node, |_| true)
}

/// Return the span of the first use of a `Context` in `node` that can panic while `guard`
/// is alive. The search includes closure bodies.
pub fn first_risky_context_use<'tcx>(
    cx: &LateContext<'tcx>,
    node: impl Visitable<'tcx>,
    guard: &Guard,
) -> Option<Span> {
    first_context_use_where(cx, node, |e| {
        let Some(call) = pliron_deref_call(cx, e) else {
            return true;
        };
        // A shared borrow does not conflict with a `Ref`.
        let shared_with_ref = guard.kind == "Ref" && call.shared;
        // An object of a different IR entity type is in a different `RefCell`.
        let other_entity = call.entity.is_some_and(|entity| entity != guard.entity);
        !(shared_with_ref || other_entity)
    })
}

/// Return the span of the first expression in `node` that is a `Context` (or a reference to
/// it), and for which `filter` returns true.
fn first_context_use_where<'tcx>(
    cx: &LateContext<'tcx>,
    node: impl Visitable<'tcx>,
    filter: impl Fn(&'tcx Expr<'tcx>) -> bool,
) -> Option<Span> {
    for_each_expr(cx.tcx, node, |e| {
        if let ExprKind::Path(_) = e.kind
            && is_context(cx, cx.typeck_results().expr_ty(e))
            && filter(e)
        {
            ControlFlow::Break(e.span)
        } else {
            ControlFlow::Continue(())
        }
    })
}

/// A call of a pliron `deref`, `try_deref`, `deref_mut` or `try_deref_mut` method.
struct DerefCall {
    /// The call only borrows the object for shared access.
    shared: bool,
    /// The IR entity borrowed by the call, if the receiver is a `Ptr` of an IR entity.
    entity: Option<&'static str>,
}

/// If `e` is the `Context` argument of a pliron `deref`, `try_deref`, `deref_mut` or
/// `try_deref_mut` method, describe that call.
fn pliron_deref_call<'tcx>(cx: &LateContext<'tcx>, e: &'tcx Expr<'tcx>) -> Option<DerefCall> {
    let Node::Expr(parent) = cx.tcx.parent_hir_node(e.hir_id) else {
        return None;
    };
    let ExprKind::MethodCall(segment, receiver, args, _) = parent.kind else {
        return None;
    };
    let shared = match segment.ident.as_str() {
        "deref" | "try_deref" => true,
        "deref_mut" | "try_deref_mut" => false,
        _ => return None,
    };
    if !args.iter().any(|arg| arg.hir_id == e.hir_id)
        || !cx
            .typeck_results()
            .type_dependent_def_id(parent.hir_id)
            .is_some_and(|did| cx.tcx.crate_name(did.krate).as_str() == "pliron")
    {
        return None;
    }
    let entity = match cx.typeck_results().expr_ty(receiver).peel_refs().kind() {
        ty::Adt(_, args) if !args.is_empty() => args
            .types()
            .next()
            .and_then(|target| ir_entity_name(cx, target)),
        _ => None,
    };
    Some(DerefCall { shared, entity })
}

/// Return true if `e` uses a `Context`.
pub fn made_through_context<'tcx>(cx: &LateContext<'tcx>, e: &'tcx Expr<'tcx>) -> bool {
    first_context_use(cx, e).is_some()
}

/// If `e` makes a `Ref` or `RefMut` temporary that is used by reference
/// (auto-deref, auto-ref, `*e` or `&e`), return its description.
pub fn borrowed_guard_temporary<'tcx>(
    cx: &LateContext<'tcx>,
    e: &'tcx Expr<'tcx>,
) -> Option<Guard> {
    if !matches!(e.kind, ExprKind::Call(..) | ExprKind::MethodCall(..)) {
        return None;
    }
    let guard = refcell_guard(cx, e)?;
    let adjusted_in_place = cx
        .typeck_results()
        .expr_adjustments(e)
        .first()
        .is_some_and(|adj| {
            matches!(
                adj.kind,
                Adjust::Deref(DerefAdjustKind::Overloaded(_)) | Adjust::Borrow(_)
            )
        });
    let used_by_parent = matches!(
        cx.tcx.parent_hir_node(e.hir_id),
        Node::Expr(Expr {
            kind: ExprKind::Unary(UnOp::Deref, _) | ExprKind::AddrOf(..),
            ..
        })
    );
    (adjusted_in_place || used_by_parent).then_some(guard)
}

/// Find the `Ref` or `RefMut` temporaries in `scrutinee` that live until the end of the
/// `for`, `match`, `if let` or `while let` that owns `scrutinee`.
pub fn scrutinee_guard_temporaries<'tcx>(
    cx: &LateContext<'tcx>,
    scrutinee: &'tcx Expr<'tcx>,
) -> Vec<(&'tcx Expr<'tcx>, Guard)> {
    let mut temporaries = Vec::new();
    let _: Option<()> = for_each_expr(cx.tcx, scrutinee, |e| {
        match e.kind {
            // Each of these expressions has its own temporary scope, so a temporary in it
            // does not live until the end of the construct that owns `scrutinee`.
            ExprKind::Closure(_)
            | ExprKind::Block(..)
            | ExprKind::If(..)
            | ExprKind::Loop(..)
            | ExprKind::Match(_, _, rustc_hir::MatchSource::Normal) => {
                return ControlFlow::Continue(Descend::No);
            }
            _ => {}
        }
        if let Some(guard) = borrowed_guard_temporary(cx, e)
            && made_through_context(cx, e)
        {
            temporaries.push((e, guard));
        }
        ControlFlow::Continue(Descend::Yes)
    });
    temporaries
}
