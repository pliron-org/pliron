use crate::utils::{OP_TRAIT, pliron_trait_ids, ptr_entity};
use clippy_utils::diagnostics::span_lint_and_help;
use rustc_data_structures::fx::FxHashSet;
use rustc_hir::def_id::DefId;
use rustc_hir::{Impl, Item, ItemKind};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty;
use rustc_session::{declare_lint, impl_lint_pass};

declare_lint! {
    /// ### Deny `Op` types with a layout other than one `Ptr<Operation>`
    ///
    /// The lint reports a type that implements `Op` and is not
    /// `#[repr(transparent)] struct MyOp { op: Ptr<Operation> }`.
    /// pliron relies on an `Op` having exactly this layout.
    /// It is suggested to always use the `#[pliron_op]` macro.
    ///
    /// ### Code reported by the lint
    ///
    /// ```rust,ignore
    /// #[derive(Clone)]
    /// struct MyOp {
    ///     op: Ptr<Operation>,
    ///     extra: u32,
    /// }
    ///
    /// impl Op for MyOp { /* ... */ }
    /// ```
    ///
    /// ### Corrected code
    ///
    /// ```rust,ignore
    /// #[pliron_op(name = "test.my_op", format, verifier = "succ")]
    /// struct MyOp {}
    /// ```
    pub OP_LAYOUT,
    Deny,
    "an `Op` type has a layout other than `#[repr(transparent)] struct { op: Ptr<Operation> }`"
}

#[derive(Default)]
pub struct OpLayout {
    /// Types already reported. A generic type can implement `Op` more than once.
    reported: FxHashSet<DefId>,
}

impl_lint_pass!(OpLayout => [OP_LAYOUT]);

impl<'tcx> LateLintPass<'tcx> for OpLayout {
    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx Item<'tcx>) {
        if !matches!(
            item.kind,
            ItemKind::Impl(Impl {
                of_trait: Some(_),
                ..
            })
        ) {
            return;
        }
        let impl_id = item.owner_id.to_def_id();
        let Some(trait_ref) = cx.tcx.impl_opt_trait_ref(impl_id) else {
            return;
        };
        let trait_ref = trait_ref.instantiate_identity().skip_norm_wip();
        if !pliron_trait_ids(cx, OP_TRAIT).contains(&trait_ref.def_id) {
            return;
        }
        // The implementing type, with the arguments of this `impl` (for example `MyOp<u32>`).
        let self_ty = trait_ref.self_ty();
        if has_op_layout(cx, self_ty) {
            return;
        }
        let (span, name) = match self_ty.kind() {
            ty::Adt(adt, _) => {
                if !self.reported.insert(adt.did()) {
                    return;
                }
                (
                    cx.tcx.def_span(adt.did()),
                    cx.tcx.item_name(adt.did()).to_string(),
                )
            }
            _ => (item.span, self_ty.to_string()),
        };
        span_lint_and_help(
            cx,
            OP_LAYOUT,
            span,
            format!(
                "the `Op` `{name}` is not `#[repr(transparent)] struct {{ op: Ptr<Operation> }}`"
            ),
            None,
            "define the `Op` with `#[pliron_op]`",
        );
    }
}

/// Return true if `ty` is `#[repr(transparent)] struct { op: Ptr<Operation> }`.
fn has_op_layout<'tcx>(cx: &LateContext<'tcx>, ty: ty::Ty<'tcx>) -> bool {
    let ty::Adt(adt, args) = ty.kind() else {
        return false;
    };
    if !adt.is_struct() || !adt.repr().transparent() {
        return false;
    }
    let mut fields = adt.all_fields();
    let (Some(field), None) = (fields.next(), fields.next()) else {
        return false;
    };
    if field.name.as_str() != "op" {
        return false;
    }
    ptr_entity(cx, field.ty(cx.tcx, args).skip_norm_wip()) == Some("Operation")
}
