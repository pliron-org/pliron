// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Conversion of [LLVM attributes](crate::llvm_attrs) to and from LLVM-IR.

use pliron::{std_deps::sync::LazyLock, utils::table::HMap};
use thiserror::Error;

use crate::llvm_sys::core::llvm_enum_attribute_kind;

/// Payload held by an LLVM enum attribute.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LlvmAttrShape {
    /// No payload, such as `nounwind`.
    Enum,
    /// An integer payload, such as `alignstack(16)`.
    Int,
    /// A type payload, such as `byval(%struct.S)`.
    Type,
    /// A constant range, such as `range(i32 0, 10)`. pliron does not model this.
    ConstantRange,
    /// A list of constant ranges, such as `initializes((0, 4))`. This isn't modelled yet.
    ConstantRangeList,
}

/// Every enum attribute that LLVM defines in `llvm/IR/Attributes.td`, with its payload shape.
///
/// LLVM's `Attribute::getNameFromAttrKind` is not in the C API.
/// [llvm_enum_attribute_info] here provides the same functionality.
///
/// When this table does not match LLVM exactly:
///   1. A name not defined by LLVM is left out of the reverse map.
///      Since LLVM never reports that kind id, nothing is lost.
///   2. A kind id not in this table makes [llvm_enum_attribute_info] return `None`.
///      The attribute is later dropped with a warning.
///
/// Regenerate with:
/// ```sh
/// tr '\n' ' ' < llvm/include/llvm/IR/Attributes.td \
///   | grep -oE 'def [A-Za-z0-9_]+ *: *(EnumAttr|IntAttr|TypeAttr|ConstantRangeAttr|ConstantRangeListAttr)<"[^"]+"' \
///   | sed -E 's/def [A-Za-z0-9_]+ *: *([A-Za-z]+)Attr<"([^"]+)"/    ("\2", \1),/' \
///   | LC_ALL=C sort -u
/// ```
const LLVM_ENUM_ATTRIBUTES: &[(&str, LlvmAttrShape)] = {
    use LlvmAttrShape::*;
    &[
        ("align", Int),
        ("alignstack", Int),
        ("allocalign", Enum),
        ("allockind", Int),
        ("allocptr", Enum),
        ("allocsize", Int),
        ("alwaysinline", Enum),
        ("builtin", Enum),
        ("byref", Type),
        ("byval", Type),
        ("captures", Int),
        ("cold", Enum),
        ("convergent", Enum),
        ("coro_elide_safe", Enum),
        ("coro_only_destroy_when_complete", Enum),
        ("dead_on_return", Int),
        ("dead_on_unwind", Enum),
        ("denormal_fpenv", Int),
        ("dereferenceable", Int),
        ("dereferenceable_or_null", Int),
        ("disable_sanitizer_instrumentation", Enum),
        ("elementtype", Type),
        ("flatten", Enum),
        ("fn_ret_thunk_extern", Enum),
        ("hot", Enum),
        ("hybrid_patchable", Enum),
        ("immarg", Enum),
        ("inalloca", Type),
        ("initializes", ConstantRangeList),
        ("inlinehint", Enum),
        ("inreg", Enum),
        ("jumptable", Enum),
        ("memory", Int),
        ("minsize", Enum),
        ("mustprogress", Enum),
        ("naked", Enum),
        ("nest", Enum),
        ("noalias", Enum),
        ("nobuiltin", Enum),
        ("nocallback", Enum),
        ("nocf_check", Enum),
        ("nocreateundeforpoison", Enum),
        ("nodivergencesource", Enum),
        ("noduplicate", Enum),
        ("noext", Enum),
        ("nofpclass", Int),
        ("nofree", Enum),
        ("noimplicitfloat", Enum),
        ("noinline", Enum),
        ("noipa", Enum),
        ("nomerge", Enum),
        ("nonlazybind", Enum),
        ("nonnull", Enum),
        ("nooutline", Enum),
        ("noprofile", Enum),
        ("norecurse", Enum),
        ("noredzone", Enum),
        ("noreturn", Enum),
        ("nosanitize_bounds", Enum),
        ("nosanitize_coverage", Enum),
        ("nosync", Enum),
        ("noundef", Enum),
        ("nounwind", Enum),
        ("null_pointer_is_valid", Enum),
        ("optdebug", Enum),
        ("optforfuzzing", Enum),
        ("optnone", Enum),
        ("optsize", Enum),
        ("preallocated", Type),
        ("presplitcoroutine", Enum),
        ("range", ConstantRange),
        ("readnone", Enum),
        ("readonly", Enum),
        ("returned", Enum),
        ("returns_twice", Enum),
        ("safestack", Enum),
        ("sanitize_address", Enum),
        ("sanitize_alloc_token", Enum),
        ("sanitize_hwaddress", Enum),
        ("sanitize_memory", Enum),
        ("sanitize_memtag", Enum),
        ("sanitize_numerical_stability", Enum),
        ("sanitize_realtime", Enum),
        ("sanitize_realtime_blocking", Enum),
        ("sanitize_thread", Enum),
        ("sanitize_type", Enum),
        ("shadowcallstack", Enum),
        ("signext", Enum),
        ("skipprofile", Enum),
        ("speculatable", Enum),
        ("speculative_load_hardening", Enum),
        ("sret", Type),
        ("ssp", Enum),
        ("sspreq", Enum),
        ("sspstrong", Enum),
        ("strictfp", Enum),
        ("swiftasync", Enum),
        ("swifterror", Enum),
        ("swiftself", Enum),
        ("uwtable", Int),
        ("vscale_range", Int),
        ("willreturn", Enum),
        ("writable", Enum),
        ("writeonly", Enum),
        ("zeroext", Enum),
    ]
};

/// Reverse of [llvm_enum_attribute_kind], over [LLVM_ENUM_ATTRIBUTES].
static LLVM_ENUM_ATTRIBUTE_KIND_INFO: LazyLock<HMap<u32, (&'static str, LlvmAttrShape)>> =
    LazyLock::new(|| {
        LLVM_ENUM_ATTRIBUTES
            .iter()
            .filter_map(|(name, shape)| {
                llvm_enum_attribute_kind(name).map(|kind| (kind, (*name, *shape)))
            })
            .collect()
    });

/// The name and payload shape of the enum attribute `kind`.
pub fn llvm_enum_attribute_info(kind: u32) -> Option<(&'static str, LlvmAttrShape)> {
    LLVM_ENUM_ATTRIBUTE_KIND_INFO.get(&kind).copied()
}

/// Errors when LLVM attributes are converted to LLVM-IR.
#[derive(Error, Debug)]
pub enum ToLlvmAttrErr {
    #[error("LLVM attribute \"{name}\" of kind {shape:?} cannot hold the given payload")]
    PayloadMismatch {
        /// The name of the attribute.
        name: String,
        /// The payload that LLVM lets the attribute hold.
        shape: LlvmAttrShape,
    },
}

/// Conversion of LLVM attributes from LLVM-IR, a companion to [crate::from_llvm_ir].
pub mod from_llvm_ir {
    use pliron::{context::Context, result::Result};

    use crate::{
        from_llvm_ir::ConversionContext,
        llvm_attrs::{LlvmAttrValue, LlvmAttributesAttr},
        llvm_sys::core::{
            LLVMAttribute, llvm_get_enum_attribute_kind, llvm_get_enum_attribute_value,
            llvm_get_string_attribute_kind, llvm_get_string_attribute_value,
            llvm_get_type_attribute_value, llvm_is_enum_attribute, llvm_is_string_attribute,
            llvm_is_type_attribute,
        },
    };

    use super::{LlvmAttrShape, llvm_enum_attribute_info};

    /// Convert every LLVM attribute in `attrs` to an entry of an [LlvmAttributesAttr].
    ///
    /// An attribute with an unknown kind id is dropped with a log warning.
    pub(crate) fn convert_llvm_attributes(
        ctx: &mut Context,
        cctx: &mut ConversionContext,
        attrs: Vec<LLVMAttribute>,
    ) -> Result<LlvmAttributesAttr> {
        let mut converted = LlvmAttributesAttr::new();
        for attr in attrs {
            if llvm_is_string_attribute(attr) {
                converted.set(
                    llvm_get_string_attribute_kind(attr),
                    LlvmAttrValue::Str(llvm_get_string_attribute_value(attr)),
                );
                continue;
            }

            let kind = llvm_get_enum_attribute_kind(attr);
            let Some((name, shape)) = llvm_enum_attribute_info(kind) else {
                log::warn!("Dropping LLVM attribute with unknown kind id {kind}");
                continue;
            };

            let value = if llvm_is_type_attribute(attr) {
                LlvmAttrValue::Type(crate::from_llvm_ir::convert_type(
                    ctx,
                    cctx,
                    llvm_get_type_attribute_value(attr),
                )?)
            } else if llvm_is_enum_attribute(attr) {
                if shape == LlvmAttrShape::Int {
                    LlvmAttrValue::Int(llvm_get_enum_attribute_value(attr))
                } else {
                    LlvmAttrValue::Unit
                }
            } else {
                // A constant-range attribute, such as `range` or `initializes`.
                log::warn!("Dropping LLVM attribute \"{name}\", whose payload is unsupported");
                continue;
            };
            converted.set(name, value);
        }
        Ok(converted)
    }
}

/// Conversion of LLVM attributes to LLVM-IR, a companion to [crate::to_llvm_ir].
pub mod to_llvm_ir {
    use pliron::{context::Context, input_err_noloc, result::Result};

    use crate::{
        llvm_attrs::{LlvmAttrValue, LlvmAttributesAttr},
        llvm_sys::core::{
            LLVM_ATTRIBUTE_FUNCTION_INDEX, LLVMAttribute, LLVMAttributeIndex, LLVMContext,
            LLVMValue, llvm_create_enum_attribute, llvm_create_string_attribute,
            llvm_create_type_attribute,
        },
        to_llvm_ir::TypeConversionContext,
    };

    use super::{LlvmAttrShape, ToLlvmAttrErr, llvm_enum_attribute_info, llvm_enum_attribute_kind};

    /// Add `attrs` to `value`, at `value`'s function index.
    ///
    /// `add_attribute` is used to select b/w global values and call sites.
    ///
    /// Drops (with a log warning) an attribute with a name that is not defined in LLVM.
    pub(crate) fn add_function_attributes(
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        tcctx: &mut TypeConversionContext,
        value: LLVMValue,
        attrs: &LlvmAttributesAttr,
        add_attribute: fn(LLVMValue, LLVMAttributeIndex, LLVMAttribute),
    ) -> Result<()> {
        for (name, attr_value) in attrs.iter() {
            // A string attribute names itself, so it needs no enum attribute kind.
            if let LlvmAttrValue::Str(s) = attr_value {
                let attr = llvm_create_string_attribute(llvm_ctx, name, s);
                add_attribute(value, LLVM_ATTRIBUTE_FUNCTION_INDEX, attr);
                continue;
            }

            // Both LLVM and the shape table must know `name`.
            let info = llvm_enum_attribute_kind(name)
                .and_then(|kind| llvm_enum_attribute_info(kind).map(|(_, shape)| (kind, shape)));
            let Some((kind, shape)) = info else {
                log::warn!("Dropping LLVM attribute \"{name}\", unknown to this LLVM version");
                continue;
            };

            // Check shape to avoid LLVM abort on assertions-enabled builds.
            let attr = match (shape, attr_value) {
                (LlvmAttrShape::Enum, LlvmAttrValue::Unit) => {
                    llvm_create_enum_attribute(llvm_ctx, kind, 0)
                }
                (LlvmAttrShape::Int, LlvmAttrValue::Int(i)) => {
                    llvm_create_enum_attribute(llvm_ctx, kind, *i)
                }
                (LlvmAttrShape::Type, LlvmAttrValue::Type(ty)) => {
                    let llvm_ty = crate::to_llvm_ir::convert_type(ctx, llvm_ctx, tcctx, *ty)?;
                    llvm_create_type_attribute(llvm_ctx, kind, llvm_ty)
                }
                (LlvmAttrShape::ConstantRange | LlvmAttrShape::ConstantRangeList, _) => {
                    log::warn!("Dropping LLVM attribute \"{name}\", whose payload is unsupported");
                    continue;
                }
                _ => {
                    return input_err_noloc!(ToLlvmAttrErr::PayloadMismatch {
                        name: name.to_string(),
                        shape,
                    });
                }
            };
            add_attribute(value, LLVM_ATTRIBUTE_FUNCTION_INDEX, attr);
        }
        Ok(())
    }
}
