use crate::utils::def_path_is;
use clippy_utils::diagnostics::span_lint_and_help;
use rustc_hir::def_id::DefId;
use rustc_hir::{AmbigArg, Ty, TyKind};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::sym;

/// Definition path of the module that wraps `hashbrown`.
const HTABLE_PATH: [&str; 4] = ["pliron", "utils", "table", "htable"];

declare_lint! {
    /// ### Deny the hash tables of `std` and `hashbrown`
    ///
    /// The lint reports a use of the type `HashMap` or `HashSet` from `std` or from
    /// `hashbrown`. The iteration order of these tables is not specified.
    ///
    /// Use `HMap`, `HSet`, `IMap` or `ISet` instead.
    ///
    /// ### Code reported by the lint
    ///
    /// ```rust,ignore
    /// let mut names: hashbrown::HashMap<u32, String> = hashbrown::HashMap::new();
    /// ```
    ///
    /// ### Corrected code
    ///
    /// ```rust,ignore
    /// let mut names: HMap<u32, String> = HMap::new();
    /// ```
    pub RAW_HASH_TABLE,
    Deny,
    "use of a `HashMap` or a `HashSet` from `std` or `hashbrown`"
}

declare_lint_pass!(RawHashTable => [RAW_HASH_TABLE]);

impl<'tcx> LateLintPass<'tcx> for RawHashTable {
    fn check_ty(&mut self, cx: &LateContext<'tcx>, ty: &'tcx Ty<'tcx, AmbigArg>) {
        let TyKind::Path(ref qpath) = ty.kind else {
            return;
        };
        let Some(did) = cx.qpath_res(qpath, ty.hir_id).opt_def_id() else {
            return;
        };
        if ty.span.in_external_macro(cx.sess().source_map()) {
            return;
        }
        let module = cx.tcx.parent_module(ty.hir_id).to_def_id();
        if def_path_is(cx, module, &HTABLE_PATH) {
            return;
        }
        let Some((table, unordered, ordered)) = raw_hash_table(cx, did) else {
            return;
        };
        span_lint_and_help(
            cx,
            RAW_HASH_TABLE,
            ty.span,
            format!("use of `{table}`"),
            None,
            format!("use `{unordered}`, or `{ordered}` for a deterministic iteration order"),
        );
    }
}

/// Name of a hash table, followed by the names of its pliron replacements: one without and
/// one with a deterministic iteration order.
type RawTable = (&'static str, &'static str, &'static str);

/// If `did` is a `HashMap` or a `HashSet` from `std` or `hashbrown`, return its name and
/// the names of its replacements in pliron.
fn raw_hash_table(cx: &LateContext<'_>, did: DefId) -> Option<RawTable> {
    match cx.tcx.get_diagnostic_name(did) {
        Some(sym::HashMap) => return Some(("std::collections::HashMap", "HMap", "IMap")),
        Some(sym::HashSet) => return Some(("std::collections::HashSet", "HSet", "ISet")),
        _ => {}
    }
    if def_path_is(cx, did, &["hashbrown", "map", "HashMap"]) {
        Some(("hashbrown::HashMap", "HMap", "IMap"))
    } else if def_path_is(cx, did, &["hashbrown", "set", "HashSet"]) {
        Some(("hashbrown::HashSet", "HSet", "ISet"))
    } else {
        None
    }
}
