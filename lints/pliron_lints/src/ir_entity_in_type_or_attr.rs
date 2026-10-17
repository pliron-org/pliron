use crate::utils::{
    ATTRIBUTE_TRAIT, OP_TRAIT, TYPE_TRAIT, implements_pliron_trait, pliron_trait_ids, ptr_entity,
};
use clippy_utils::diagnostics::span_lint_and_then;
use rustc_data_structures::fx::FxHashSet;
use rustc_hir::def_id::DefId;
use rustc_hir::{Item, ItemKind, VariantData};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty::{self, Ty, Unnormalized};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::sym;
use rustc_trait_selection::traits::supertrait_def_ids;

declare_lint! {
    /// ### Deny IR entities in types and attributes
    ///
    /// The lint reports a field of a pliron type or an attribute
    /// (a struct or an enum that implements `Type` or `Attribute`)
    /// that holds an `Operation`, a `BasicBlock` or a `Region`.
    ///
    /// ### Code reported by the lint
    ///
    /// ```rust,ignore
    /// #[pliron_attr(name = "test.target", format)]
    /// struct TargetAttr {
    ///     target: Ptr<Operation>,
    /// }
    /// ```
    ///
    /// ### Corrected code
    ///
    /// ```rust,ignore
    /// #[pliron_attr(name = "test.target", format)]
    /// struct TargetAttr {
    ///     target: Identifier,
    /// }
    /// ```
    pub IR_ENTITY_IN_TYPE_OR_ATTR,
    Deny,
    "a type or an attribute holds an `Operation`, a `BasicBlock` or a `Region`"
}

declare_lint_pass!(IrEntityInTypeOrAttr => [IR_ENTITY_IN_TYPE_OR_ATTR]);

impl<'tcx> LateLintPass<'tcx> for IrEntityInTypeOrAttr {
    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx Item<'tcx>) {
        let variants: Vec<&VariantData<'tcx>> = match &item.kind {
            ItemKind::Struct(_, _, data) => vec![data],
            ItemKind::Enum(_, _, def) => def.variants.iter().map(|v| &v.data).collect(),
            _ => return,
        };
        let def_id = item.owner_id.to_def_id();
        let ty = cx
            .tcx
            .type_of(def_id)
            .instantiate_identity()
            .skip_norm_wip();
        let kind = if implements_pliron_trait(cx, ty, TYPE_TRAIT) {
            "type"
        } else if implements_pliron_trait(cx, ty, ATTRIBUTE_TRAIT) {
            "attribute"
        } else {
            return;
        };
        let name = cx.tcx.item_name(def_id);
        for field in variants.iter().flat_map(|data| data.fields()) {
            let field_ty = normalize(cx, cx.tcx.type_of(field.def_id).instantiate_identity());
            let mut through = Vec::new();
            let mut visited = FxHashSet::default();
            let Some(found) = find_ir_entity(cx, field_ty, &mut through, &mut visited) else {
                continue;
            };
            span_lint_and_then(
                cx,
                IR_ENTITY_IN_TYPE_OR_ATTR,
                field.ty.span,
                format!(
                    "the {kind} `{name}` holds a `{found}` in field `{}`",
                    field.ident
                ),
                |diag| {
                    if !through.is_empty() {
                        diag.note(format!("through `{}`", through.join("` → `")));
                    }
                    diag.help(
                        "refer to the IR object by name (for example a symbol), \
                         or keep the data in the operation",
                    );
                },
            );
        }
    }
}

/// Return true if the trait `trait_id` is `pliron::op::Op` or extends it.
fn is_op_trait(cx: &LateContext<'_>, trait_id: DefId) -> bool {
    let op_traits = pliron_trait_ids(cx, OP_TRAIT);
    supertrait_def_ids(cx.tcx, trait_id).any(|id| op_traits.contains(&id))
}

/// The maximum number of nested structs and enums that the search goes through.
const MAX_DEPTH: usize = 32;

/// Normalize `ty`, so that an associated type (for example `T::Data`) becomes the actual type.
fn normalize<'tcx>(cx: &LateContext<'tcx>, ty: Unnormalized<'tcx, Ty<'tcx>>) -> Ty<'tcx> {
    cx.tcx
        .try_normalize_erasing_regions(cx.typing_env(), ty)
        .unwrap_or(ty.skip_norm_wip())
}

/// Find an IR entity held by a value of type `ty`, and return its description.
/// `through` collects the names of the structs and enums whose fields lead to it.
fn find_ir_entity<'tcx>(
    cx: &LateContext<'tcx>,
    ty: Ty<'tcx>,
    through: &mut Vec<String>,
    visited: &mut FxHashSet<Ty<'tcx>>,
) -> Option<String> {
    match ty.kind() {
        ty::Adt(adt, args) => {
            if adt.is_phantom_data() {
                return None;
            }
            if let Some(entity) = ptr_entity(cx, ty) {
                return Some(format!("Ptr<{entity}>"));
            }
            let krate = cx.tcx.crate_name(adt.did().krate);
            if matches!(krate, sym::core | sym::alloc | sym::std) {
                // The fields of a standard library type are its internals, so search its
                // generic arguments.
                return args
                    .types()
                    .find_map(|arg| find_ir_entity(cx, arg, through, visited));
            }
            // Limit the depth for types with growing generic arguments.
            if through.len() >= MAX_DEPTH || !visited.insert(ty) {
                return None;
            }
            through.push(cx.tcx.item_name(adt.did()).to_string());
            let found = adt.all_fields().find_map(|field| {
                find_ir_entity(cx, normalize(cx, field.ty(cx.tcx, args)), through, visited)
            });
            if found.is_none() {
                through.pop();
            }
            found
        }
        ty::Dynamic(predicates, ..) => predicates
            .principal_def_id()
            .filter(|&trait_id| is_op_trait(cx, trait_id))
            .map(|trait_id| format!("dyn {}", cx.tcx.item_name(trait_id))),
        ty::Ref(_, inner, _) | ty::RawPtr(inner, _) | ty::Array(inner, _) | ty::Slice(inner) => {
            find_ir_entity(cx, *inner, through, visited)
        }
        ty::Tuple(elements) => elements
            .iter()
            .find_map(|element| find_ir_entity(cx, element, through, visited)),
        _ => None,
    }
}
