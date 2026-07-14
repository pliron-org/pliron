//! Translate from pliron's LLVM dialect to LLVM-IR

use std::collections::hash_map;

use llvm_sys::{
    LLVMAtomicOrdering, LLVMAtomicRMWBinOp, LLVMInlineAsmDialect, LLVMIntPredicate, LLVMLinkage,
    LLVMRealPredicate,
};
use pliron::{
    attribute::{Attribute, attr_cast},
    basic_block::BasicBlock,
    builtin::{
        attr_interfaces::FloatAttr,
        attributes::{FPDoubleAttr, FPSingleAttr, IntegerAttr, StringAttr},
        op_interfaces::{
            AtMostOneRegionInterface, BranchOpInterface, CallOpCallable, CallOpInterface,
            OneOpdInterface, OneResultInterface, SingleBlockRegionInterface, SymbolOpInterface,
        },
        ops::ModuleOp,
        type_interfaces::FunctionTypeInterface,
        types::{FP16Type, FP32Type, FP64Type, IntegerType},
    },
    common_traits::Named,
    context::{Context, Ptr},
    derive::{attr_interface, attr_interface_impl},
    graph::traversals::region::topological_order,
    identifier::Identifier,
    input_err, input_err_noloc, input_error, input_error_noloc,
    linked_list::{ContainsLinkedList, LinkedList},
    location::Located,
    op::{Op, op_cast},
    operation::Operation,
    printable::Printable,
    result::Result,
    r#type::{Type, TypeHandle, Typed, type_cast},
    utils::apint::APInt,
    value::{DefiningEntity, Value},
};

use pliron::derive::{op_interface, op_interface_impl, type_interface, type_interface_impl};
use rustc_hash::FxHashMap;
use thiserror::Error;

use crate::{
    attributes::{
        AtomicOrderingAttr, AtomicRmwKindAttr, FCmpPredicateAttr, ICmpPredicateAttr, LinkageAttr,
    },
    llvm_sys::core::{
        LLVMBasicBlock, LLVMBuilder, LLVMContext, LLVMModule, LLVMType, LLVMValue,
        instruction_iter, llvm_add_case, llvm_add_function, llvm_add_global_in_address_space,
        llvm_add_incoming, llvm_append_basic_block_in_context, llvm_array_type2, llvm_build_add,
        llvm_build_addrspacecast, llvm_build_and, llvm_build_array_alloca, llvm_build_ashr,
        llvm_build_atomic_cmpxchg, llvm_build_atomic_rmw, llvm_build_bitcast, llvm_build_br,
        llvm_build_call2, llvm_build_cond_br, llvm_build_extract_element, llvm_build_extract_value,
        llvm_build_fadd, llvm_build_fcmp, llvm_build_fdiv, llvm_build_fence, llvm_build_fmul,
        llvm_build_fpext, llvm_build_fptosi, llvm_build_fptoui, llvm_build_fptrunc,
        llvm_build_freeze, llvm_build_frem, llvm_build_fsub, llvm_build_gep2, llvm_build_icmp,
        llvm_build_insert_element, llvm_build_insert_value, llvm_build_int_to_ptr,
        llvm_build_load2, llvm_build_lshr, llvm_build_mul, llvm_build_or, llvm_build_phi,
        llvm_build_ptr_to_int, llvm_build_ret, llvm_build_ret_void, llvm_build_sdiv,
        llvm_build_select, llvm_build_sext, llvm_build_shl, llvm_build_shuffle_vector,
        llvm_build_sitofp, llvm_build_srem, llvm_build_store, llvm_build_sub, llvm_build_switch,
        llvm_build_trunc, llvm_build_udiv, llvm_build_uitofp, llvm_build_unreachable,
        llvm_build_urem, llvm_build_va_arg, llvm_build_xor, llvm_build_zext,
        llvm_can_value_use_fast_math_flags, llvm_clear_insertion_position, llvm_const_int,
        llvm_const_null, llvm_const_real, llvm_const_vector, llvm_double_type_in_context,
        llvm_float_type_in_context, llvm_function_type, llvm_get_inline_asm,
        llvm_get_named_function, llvm_get_param, llvm_get_poison, llvm_get_sync_scope_id,
        llvm_get_undef, llvm_half_type_in_context, llvm_int_type_in_context, llvm_is_a,
        llvm_lookup_intrinsic_id, llvm_pointer_type_in_context, llvm_position_builder_at_end,
        llvm_scalable_vector_type, llvm_set_alignment, llvm_set_atomic_sync_scope_id,
        llvm_set_fast_math_flags, llvm_set_initializer, llvm_set_linkage, llvm_set_nneg,
        llvm_set_ordering, llvm_struct_create_named, llvm_struct_set_body,
        llvm_struct_type_in_context, llvm_type_of, llvm_vector_type, llvm_void_type_in_context,
    },
    op_interfaces::{
        AlignableOpInterface, FastMathFlags, IsDeclaration, LlvmSymbolName, NNegFlag,
        PointerTypeResult,
    },
    ops::{
        AShrOp, AddOp, AddrSpaceCastOp, AddressOfOp, AllocaOp, AndOp, AtomicCmpxchgOp,
        AtomicLoadOp, AtomicRmwOp, AtomicStoreOp, BitcastOp, BrOp, CallIntrinsicOp, CallOp,
        CondBrOp, ConstantOp, ExtractElementOp, ExtractValueOp, FAddOp, FCmpOp, FDivOp, FMulOp,
        FPExtOp, FPToSIOp, FPToUIOp, FPTruncOp, FRemOp, FSubOp, FenceOp, FreezeOp, FuncOp,
        GetElementPtrOp, GlobalOp, ICmpOp, InlineAsmOp, InsertElementOp, InsertValueOp, IntToPtrOp,
        LShrOp, LoadOp, MulOp, OrOp, PoisonOp, PtrToIntOp, ReturnOp, SDivOp, SExtOp, SIToFPOp,
        SRemOp, SelectOp, ShlOp, ShuffleVectorOp, StoreOp, SubOp, SwitchOp, TruncOp, UDivOp,
        UIToFPOp, URemOp, UndefOp, UnreachableOp, VAArgOp, XorOp, ZExtOp, ZeroOp,
    },
    types::{ArrayType, FuncType, PointerType, StructType, VectorType, VoidType},
};

/// Mapping from pliron entities to LLVM entities.
pub struct ConversionContext<'a> {
    // The current LLVMModule being converted to.
    cur_llvm_module: &'a LLVMModule,
    // A map from pliron Values to LLVM Values.
    value_map: FxHashMap<Value, LLVMValue>,
    // A map from pliron basic blocks to LLVM.
    block_map: FxHashMap<Ptr<BasicBlock>, LLVMBasicBlock>,
    // A map from pliron functions to LLVM functions.
    function_map: FxHashMap<Identifier, LLVMValue>,
    // A map from pliron globals to LLVM globals.
    globals_map: FxHashMap<Identifier, LLVMValue>,
    // A map from pliron StructTypes to LLVM StructTypes.
    structs_map: FxHashMap<Identifier, LLVMType>,
    // Type cache to avoid redundant conversions.
    type_cache: FxHashMap<TypeHandle, LLVMType>,
    // The active LLVM builder.
    builder: LLVMBuilder,
    // Scratch builder in a scratch function for attempting to evaluate constants.
    scratch_builder: LLVMBuilder,
}

impl<'a> ConversionContext<'a> {
    pub fn new(llvm_ctx: &'a LLVMContext, cur_llvm_module: &'a LLVMModule) -> Self {
        Self {
            cur_llvm_module,
            value_map: FxHashMap::default(),
            block_map: FxHashMap::default(),
            function_map: FxHashMap::default(),
            globals_map: FxHashMap::default(),
            structs_map: FxHashMap::default(),
            type_cache: FxHashMap::default(),
            builder: LLVMBuilder::new(llvm_ctx),
            scratch_builder: LLVMBuilder::new(llvm_ctx),
        }
    }

    pub fn clear_per_function_data(&mut self) {
        self.value_map.clear();
        self.block_map.clear();
        llvm_clear_insertion_position(&self.builder);
    }
}

#[derive(Error, Debug)]
pub enum ToLLVMErr {
    #[error("Type {0} does not have a conversion to LLVM type implemented")]
    MissingTypeConversion(String),
    #[error("Operation {0} does not have a conversion to LLVM instruction implemented")]
    MissingOpConversion(String),
    #[error("Definition for value {0} not seen yet")]
    UndefinedValue(String),
    #[error("Block definition {0} not seen yet")]
    UndefinedBlock(String),
    #[error("Number of block args in the source dialect equal the number of PHIs in target IR")]
    NumBlockArgsNumPhisMismatch,
    #[error("ConstantOp must have integer or float value")]
    ConstOpNotIntOrFloat,
    #[error(
        "Insert/Extract value instructions must specify exactly one index, an LLVM-C API limitation"
    )]
    InsertExtractValueIndices,
    #[error("GlobalOp Initializer region does not terminate with a return with value")]
    GlobalOpInitializerRegionBadReturn,
    #[error("Cannot evaluate value to a constant")]
    CannotEvaluateToConst,
}

pub fn convert_ipredicate(pred: ICmpPredicateAttr) -> LLVMIntPredicate {
    match pred {
        ICmpPredicateAttr::EQ => LLVMIntPredicate::LLVMIntEQ,
        ICmpPredicateAttr::NE => LLVMIntPredicate::LLVMIntNE,
        ICmpPredicateAttr::UGT => LLVMIntPredicate::LLVMIntUGT,
        ICmpPredicateAttr::UGE => LLVMIntPredicate::LLVMIntUGE,
        ICmpPredicateAttr::ULT => LLVMIntPredicate::LLVMIntULT,
        ICmpPredicateAttr::ULE => LLVMIntPredicate::LLVMIntULE,
        ICmpPredicateAttr::SGT => LLVMIntPredicate::LLVMIntSGT,
        ICmpPredicateAttr::SGE => LLVMIntPredicate::LLVMIntSGE,
        ICmpPredicateAttr::SLT => LLVMIntPredicate::LLVMIntSLT,
        ICmpPredicateAttr::SLE => LLVMIntPredicate::LLVMIntSLE,
    }
}

pub fn convert_fpredicate(pred: FCmpPredicateAttr) -> LLVMRealPredicate {
    match pred {
        FCmpPredicateAttr::False => LLVMRealPredicate::LLVMRealPredicateFalse,
        FCmpPredicateAttr::OEQ => LLVMRealPredicate::LLVMRealOEQ,
        FCmpPredicateAttr::OGT => LLVMRealPredicate::LLVMRealOGT,
        FCmpPredicateAttr::OGE => LLVMRealPredicate::LLVMRealOGE,
        FCmpPredicateAttr::OLT => LLVMRealPredicate::LLVMRealOLT,
        FCmpPredicateAttr::OLE => LLVMRealPredicate::LLVMRealOLE,
        FCmpPredicateAttr::ONE => LLVMRealPredicate::LLVMRealONE,
        FCmpPredicateAttr::ORD => LLVMRealPredicate::LLVMRealORD,
        FCmpPredicateAttr::UNO => LLVMRealPredicate::LLVMRealUNO,
        FCmpPredicateAttr::UEQ => LLVMRealPredicate::LLVMRealUEQ,
        FCmpPredicateAttr::UGT => LLVMRealPredicate::LLVMRealUGT,
        FCmpPredicateAttr::UGE => LLVMRealPredicate::LLVMRealUGE,
        FCmpPredicateAttr::ULT => LLVMRealPredicate::LLVMRealULT,
        FCmpPredicateAttr::ULE => LLVMRealPredicate::LLVMRealULE,
        FCmpPredicateAttr::UNE => LLVMRealPredicate::LLVMRealUNE,
        FCmpPredicateAttr::True => LLVMRealPredicate::LLVMRealPredicateTrue,
    }
}

pub fn convert_linkage(linkage: LinkageAttr) -> LLVMLinkage {
    match linkage {
        LinkageAttr::ExternalLinkage => LLVMLinkage::LLVMExternalLinkage,
        LinkageAttr::AvailableExternallyLinkage => LLVMLinkage::LLVMAvailableExternallyLinkage,
        LinkageAttr::LinkOnceAnyLinkage => LLVMLinkage::LLVMLinkOnceAnyLinkage,
        LinkageAttr::LinkOnceODRLinkage => LLVMLinkage::LLVMLinkOnceODRLinkage,
        LinkageAttr::WeakAnyLinkage => LLVMLinkage::LLVMWeakAnyLinkage,
        LinkageAttr::WeakODRLinkage => LLVMLinkage::LLVMWeakODRLinkage,
        LinkageAttr::AppendingLinkage => LLVMLinkage::LLVMAppendingLinkage,
        LinkageAttr::InternalLinkage => LLVMLinkage::LLVMInternalLinkage,
        LinkageAttr::PrivateLinkage => LLVMLinkage::LLVMPrivateLinkage,
        LinkageAttr::DLLImportLinkage => LLVMLinkage::LLVMDLLImportLinkage,
        LinkageAttr::DLLExportLinkage => LLVMLinkage::LLVMDLLExportLinkage,
        LinkageAttr::ExternalWeakLinkage => LLVMLinkage::LLVMExternalWeakLinkage,
        LinkageAttr::GhostLinkage => LLVMLinkage::LLVMGhostLinkage,
        LinkageAttr::CommonLinkage => LLVMLinkage::LLVMCommonLinkage,
        LinkageAttr::LinkOnceODRAutoHideLinkage => LLVMLinkage::LLVMLinkOnceODRAutoHideLinkage,
        LinkageAttr::LinkerPrivateLinkage => LLVMLinkage::LLVMLinkerPrivateLinkage,
        LinkageAttr::LinkerPrivateWeakLinkage => LLVMLinkage::LLVMLinkerPrivateWeakLinkage,
    }
}

#[::pliron::linkme::distributed_slice]
#[linkme(crate = pliron::linkme)]
pub static TEST: [u64];

/// Convert a float attribute to fp64 (since LLVM's C-API pretty much restricts us to that).
#[attr_interface]
trait FloatAttrToFP64: FloatAttr {
    fn to_fp64(&self) -> f64;
    fn verify(_attr: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[attr_interface_impl]
impl FloatAttrToFP64 for FPSingleAttr {
    fn to_fp64(&self) -> f64 {
        Into::<f32>::into(self.clone()) as f64
    }
}

#[attr_interface_impl]
impl FloatAttrToFP64 for FPDoubleAttr {
    fn to_fp64(&self) -> f64 {
        Into::<f64>::into(self.clone())
    }
}

/// A type that implements this is convertible to an [LLVMType].
#[type_interface]
trait ToLLVMType {
    /// Convert from pliron [Type] to [LLVMType].
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMType>;

    fn verify(_type: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// An [Op] that implements this is convertible to an [LLVMValue].
#[op_interface]
trait ToLLVMValue {
    /// Convert from pliron [Op] to [LLVMValue].
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue>;

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[type_interface_impl]
impl ToLLVMType for IntegerType {
    fn convert(
        &self,
        _ctx: &Context,
        llvm_ctx: &LLVMContext,
        _cctx: &mut ConversionContext,
    ) -> Result<LLVMType> {
        Ok(llvm_int_type_in_context(llvm_ctx, self.width()))
    }
}

#[type_interface_impl]
impl ToLLVMType for ArrayType {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMType> {
        let elem_ty = convert_type(ctx, llvm_ctx, cctx, self.elem_type())?;
        Ok(llvm_array_type2(elem_ty, self.size()))
    }
}

#[type_interface_impl]
impl ToLLVMType for FuncType {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMType> {
        let args_tys: Vec<_> = self
            .arg_types()
            .iter()
            .map(|ty| convert_type(ctx, llvm_ctx, cctx, *ty))
            .collect::<Result<_>>()?;
        let ret_ty = convert_type(ctx, llvm_ctx, cctx, self.result_type())?;
        Ok(llvm_function_type(ret_ty, &args_tys, self.is_var_arg()))
    }
}

#[type_interface_impl]
impl ToLLVMType for VoidType {
    fn convert(
        &self,
        _ctx: &Context,
        llvm_ctx: &LLVMContext,
        _cctx: &mut ConversionContext,
    ) -> Result<LLVMType> {
        Ok(llvm_void_type_in_context(llvm_ctx))
    }
}

#[type_interface_impl]
impl ToLLVMType for PointerType {
    fn convert(
        &self,
        _ctx: &Context,
        llvm_ctx: &LLVMContext,
        _cctx: &mut ConversionContext,
    ) -> Result<LLVMType> {
        Ok(llvm_pointer_type_in_context(llvm_ctx, self.address_space()))
    }
}

#[type_interface_impl]
impl ToLLVMType for StructType {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMType> {
        if self.is_opaque() {
            let name = self.name().expect("Opaqaue struct must have a name");
            Ok(llvm_struct_create_named(llvm_ctx, name.as_str()))
        } else {
            let field_types = self
                .fields()
                .map(|fty| convert_type(ctx, llvm_ctx, cctx, fty))
                .collect::<Result<Vec<_>>>()?;
            if let Some(name) = self.name() {
                match cctx.structs_map.entry(name) {
                    hash_map::Entry::Occupied(entry) => Ok(*entry.get()),
                    hash_map::Entry::Vacant(entry) => {
                        let str_ty = llvm_struct_create_named(llvm_ctx, entry.key());
                        llvm_struct_set_body(str_ty, &field_types, false);
                        entry.insert(str_ty);
                        Ok(str_ty)
                    }
                }
            } else {
                Ok(llvm_struct_type_in_context(llvm_ctx, &field_types, false))
            }
        }
    }
}

#[type_interface_impl]
impl ToLLVMType for VectorType {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMType> {
        let elem_ty = convert_type(ctx, llvm_ctx, cctx, self.elem_type())?;
        let num_elems = self.num_elements();
        if self.is_scalable() {
            Ok(llvm_scalable_vector_type(elem_ty, num_elems))
        } else {
            Ok(llvm_vector_type(elem_ty, num_elems))
        }
    }
}

#[type_interface_impl]
impl ToLLVMType for FP32Type {
    fn convert(
        &self,
        _ctx: &Context,
        llvm_ctx: &LLVMContext,
        _cctx: &mut ConversionContext,
    ) -> Result<LLVMType> {
        Ok(llvm_float_type_in_context(llvm_ctx))
    }
}

#[type_interface_impl]
impl ToLLVMType for FP64Type {
    fn convert(
        &self,
        _ctx: &Context,
        llvm_ctx: &LLVMContext,
        _cctx: &mut ConversionContext,
    ) -> Result<LLVMType> {
        Ok(llvm_double_type_in_context(llvm_ctx))
    }
}

#[type_interface_impl]
impl ToLLVMType for FP16Type {
    fn convert(
        &self,
        _ctx: &Context,
        llvm_ctx: &LLVMContext,
        _cctx: &mut ConversionContext,
    ) -> Result<LLVMType> {
        Ok(llvm_half_type_in_context(llvm_ctx))
    }
}

/// Convert a pliron [Type] to [LLVMType].
pub fn convert_type(
    ctx: &Context,
    llvm_ctx: &LLVMContext,
    cctx: &mut ConversionContext,
    ty: TypeHandle,
) -> Result<LLVMType> {
    if let Some(cached) = cctx.type_cache.get(&ty) {
        return Ok(*cached);
    }
    if let Some(converter) = type_cast::<dyn ToLLVMType>(&*ty.deref(ctx)) {
        let llvm_ty = converter.convert(ctx, llvm_ctx, cctx)?;
        cctx.type_cache.insert(ty, llvm_ty);
        return Ok(llvm_ty);
    }

    input_err_noloc!(ToLLVMErr::MissingTypeConversion(
        ty.deref(ctx).get_type_id().to_string()
    ))
}

fn convert_value_operand(
    cctx: &mut ConversionContext,
    ctx: &Context,
    value: &Value,
) -> Result<LLVMValue> {
    match cctx.value_map.get(value) {
        Some(v) => Ok(*v),
        None => {
            input_err_noloc!(ToLLVMErr::UndefinedValue(value.unique_name(ctx).into()))
        }
    }
}

fn convert_block_operand(
    cctx: &mut ConversionContext,
    ctx: &Context,
    block: Ptr<BasicBlock>,
) -> Result<LLVMBasicBlock> {
    match cctx.block_map.get(&block) {
        Some(v) => Ok(*v),
        None => {
            input_err_noloc!(ToLLVMErr::UndefinedBlock(block.unique_name(ctx).into()))
        }
    }
}

macro_rules! to_llvm_value_int_bin_op {
    (
        $op_name:ident, $builder_function:ident
    ) => {
        #[pliron::derive::op_interface_impl]
        impl ToLLVMValue for $op_name {
            fn convert(
                &self,
                ctx: &Context,
                _llvm_ctx: &LLVMContext,
                cctx: &mut ConversionContext,
            ) -> Result<LLVMValue> {
                let op = self.get_operation().deref(ctx);
                let (lhs, rhs) = (op.get_operand(0), op.get_operand(1));
                let lhs = convert_value_operand(cctx, ctx, &lhs)?;
                let rhs = convert_value_operand(cctx, ctx, &rhs)?;
                Ok($builder_function(
                    &cctx.builder,
                    lhs,
                    rhs,
                    &self.get_result(ctx).unique_name(ctx),
                ))
            }
        }
    };
}

to_llvm_value_int_bin_op!(AddOp, llvm_build_add);
to_llvm_value_int_bin_op!(SubOp, llvm_build_sub);
to_llvm_value_int_bin_op!(MulOp, llvm_build_mul);
to_llvm_value_int_bin_op!(SDivOp, llvm_build_sdiv);
to_llvm_value_int_bin_op!(UDivOp, llvm_build_udiv);
to_llvm_value_int_bin_op!(URemOp, llvm_build_urem);
to_llvm_value_int_bin_op!(SRemOp, llvm_build_srem);
to_llvm_value_int_bin_op!(AndOp, llvm_build_and);
to_llvm_value_int_bin_op!(OrOp, llvm_build_or);
to_llvm_value_int_bin_op!(XorOp, llvm_build_xor);
to_llvm_value_int_bin_op!(ShlOp, llvm_build_shl);
to_llvm_value_int_bin_op!(LShrOp, llvm_build_lshr);
to_llvm_value_int_bin_op!(AShrOp, llvm_build_ashr);

#[op_interface_impl]
impl ToLLVMValue for AllocaOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_pointee_type(ctx))?;
        let size = convert_value_operand(cctx, ctx, &self.get_operand(ctx))?;
        let alloca_op = llvm_build_array_alloca(
            &cctx.builder,
            ty,
            size,
            &self.get_result(ctx).unique_name(ctx),
        );
        if let Some(alignment) = self.alignment(ctx) {
            llvm_set_alignment(alloca_op, alignment);
        }
        Ok(alloca_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for BitcastOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let arg = convert_value_operand(cctx, ctx, &self.get_operand(ctx))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let bitcast_op = llvm_build_bitcast(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(bitcast_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for AddrSpaceCastOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let arg = convert_value_operand(cctx, ctx, &self.get_operand(ctx))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let addrspacecast_op = llvm_build_addrspacecast(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(addrspacecast_op)
    }
}

fn link_succ_operands_with_phis(
    ctx: &Context,
    cctx: &mut ConversionContext,
    source_block: Ptr<BasicBlock>,
    target_block: LLVMBasicBlock,
    opds: Vec<Value>,
) -> Result<()> {
    let mut phis = vec![];
    for inst in instruction_iter(target_block) {
        if !llvm_is_a::phi_node(inst) {
            break;
        };
        phis.push(inst);
    }

    if phis.len() != opds.len() {
        return input_err!(
            source_block.deref(ctx).loc(),
            ToLLVMErr::NumBlockArgsNumPhisMismatch
        );
    }

    let source_block = convert_block_operand(cctx, ctx, source_block)?;

    for (idx, arg) in opds.iter().enumerate() {
        let arg = convert_value_operand(cctx, ctx, arg)?;
        llvm_add_incoming(phis[idx], &[arg], &[source_block]);
    }
    Ok(())
}

#[op_interface_impl]
impl ToLLVMValue for BrOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let succ = op.get_successor(0);
        let succ_llvm = convert_block_operand(cctx, ctx, succ)?;
        let branch_op = llvm_build_br(&cctx.builder, succ_llvm);

        // Link the arguments we pass to the block with the PHIs there.
        link_succ_operands_with_phis(
            ctx,
            cctx,
            op.get_container().expect("Unlinked operation"),
            succ_llvm,
            self.successor_operands(ctx, 0),
        )?;

        Ok(branch_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for CondBrOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let (true_succ, false_succ) = (op.get_successor(0), op.get_successor(1));
        let true_succ_llvm = convert_block_operand(cctx, ctx, true_succ)?;
        let false_succ_llvm = convert_block_operand(cctx, ctx, false_succ)?;
        let cond = convert_value_operand(cctx, ctx, &self.get_operand_condition(ctx))?;

        let branch_op = llvm_build_cond_br(&cctx.builder, cond, true_succ_llvm, false_succ_llvm);

        // Link the arguments we pass to the block with the PHIs there.
        link_succ_operands_with_phis(
            ctx,
            cctx,
            op.get_container().expect("Unlinked operation"),
            true_succ_llvm,
            self.successor_operands(ctx, 0),
        )?;
        link_succ_operands_with_phis(
            ctx,
            cctx,
            op.get_container().expect("Unlinked operation"),
            false_succ_llvm,
            self.successor_operands(ctx, 1),
        )?;

        Ok(branch_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for SwitchOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let cond = convert_value_operand(cctx, ctx, &self.get_operand_condition(ctx))?;
        let default_succ = convert_block_operand(cctx, ctx, self.default_dest(ctx))?;
        let switch_op = llvm_build_switch(
            &cctx.builder,
            cond,
            default_succ,
            self.cases(ctx).len() as u32,
        );

        // Link the arguments we pass to the block with the PHIs there.
        link_succ_operands_with_phis(
            ctx,
            cctx,
            op.get_container().expect("Unlinked operation"),
            default_succ,
            self.default_dest_operands(ctx),
        )?;
        for case in self.cases(ctx) {
            let succ_llvm = convert_block_operand(cctx, ctx, case.dest)?;
            link_succ_operands_with_phis(
                ctx,
                cctx,
                op.get_container().expect("Unlinked operation"),
                succ_llvm,
                case.dest_opds,
            )?;

            let int_ty = case.value.get_type();
            let int_ty_llvm = convert_type(ctx, llvm_ctx, cctx, int_ty.into())?;
            let ap_int_val: APInt = case.value.clone().into();
            let case_const_val = llvm_const_int(int_ty_llvm, ap_int_val.to_u64(), false);

            llvm_add_case(switch_op, case_const_val, succ_llvm);
        }

        Ok(switch_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for LoadOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let pointee_ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let ptr = convert_value_operand(cctx, ctx, &self.get_operand(ctx))?;
        let load_op = llvm_build_load2(
            &cctx.builder,
            pointee_ty,
            ptr,
            &self.get_result(ctx).unique_name(ctx),
        );
        if let Some(alignment) = self.alignment(ctx) {
            llvm_set_alignment(load_op, alignment);
        }
        Ok(load_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for StoreOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let value = convert_value_operand(cctx, ctx, &self.get_operand_value(ctx))?;
        let ptr = convert_value_operand(cctx, ctx, &self.get_operand_address(ctx))?;
        let store_op = llvm_build_store(&cctx.builder, value, ptr);
        if let Some(alignment) = self.alignment(ctx) {
            llvm_set_alignment(store_op, alignment);
        }
        Ok(store_op)
    }
}

/// Map a pliron [AtomicOrderingAttr] to its LLVM-C counterpart.
fn convert_atomic_ordering(o: &AtomicOrderingAttr) -> LLVMAtomicOrdering {
    match o {
        AtomicOrderingAttr::Monotonic => LLVMAtomicOrdering::LLVMAtomicOrderingMonotonic,
        AtomicOrderingAttr::Acquire => LLVMAtomicOrdering::LLVMAtomicOrderingAcquire,
        AtomicOrderingAttr::Release => LLVMAtomicOrdering::LLVMAtomicOrderingRelease,
        AtomicOrderingAttr::AcqRel => LLVMAtomicOrdering::LLVMAtomicOrderingAcquireRelease,
        AtomicOrderingAttr::SeqCst => LLVMAtomicOrdering::LLVMAtomicOrderingSequentiallyConsistent,
    }
}

/// Map a pliron [AtomicRmwKindAttr] to its LLVM-C counterpart.
fn convert_rmw_kind(k: &AtomicRmwKindAttr) -> LLVMAtomicRMWBinOp {
    match k {
        AtomicRmwKindAttr::Xchg => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpXchg,
        AtomicRmwKindAttr::Add => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpAdd,
        AtomicRmwKindAttr::Sub => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpSub,
        AtomicRmwKindAttr::And => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpAnd,
        AtomicRmwKindAttr::Nand => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpNand,
        AtomicRmwKindAttr::Or => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpOr,
        AtomicRmwKindAttr::Xor => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpXor,
        AtomicRmwKindAttr::Max => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpMax,
        AtomicRmwKindAttr::Min => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpMin,
        AtomicRmwKindAttr::UMax => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpUMax,
        AtomicRmwKindAttr::UMin => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpUMin,
        AtomicRmwKindAttr::FAdd => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpFAdd,
        AtomicRmwKindAttr::FSub => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpFSub,
        AtomicRmwKindAttr::FMax => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpFMax,
        AtomicRmwKindAttr::FMin => LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpFMin,
    }
}

#[op_interface_impl]
impl ToLLVMValue for AtomicRmwOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let (ptr_opd, val_opd) = {
            let op = self.get_operation().deref(ctx);
            (op.get_operand(0), op.get_operand(1))
        };
        let ptr = convert_value_operand(cctx, ctx, &ptr_opd)?;
        let val = convert_value_operand(cctx, ctx, &val_opd)?;
        let kind = convert_rmw_kind(
            &self
                .get_attr_llvm_rmw_kind(ctx)
                .expect("atomicrmw missing rmw kind"),
        );
        let ordering = convert_atomic_ordering(
            &self
                .get_attr_llvm_rmw_ordering(ctx)
                .expect("atomicrmw missing ordering"),
        );
        let scope = self
            .get_attr_llvm_rmw_syncscope(ctx)
            .map(|s| String::from((*s).clone()))
            .unwrap_or_default();
        let ssid = llvm_get_sync_scope_id(llvm_ctx, &scope);
        Ok(llvm_build_atomic_rmw(
            &cctx.builder,
            kind,
            ptr,
            val,
            ordering,
            ssid,
        ))
    }
}

#[op_interface_impl]
impl ToLLVMValue for AtomicCmpxchgOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let (ptr_opd, cmp_opd, new_opd) = {
            let op = self.get_operation().deref(ctx);
            (op.get_operand(0), op.get_operand(1), op.get_operand(2))
        };
        let ptr = convert_value_operand(cctx, ctx, &ptr_opd)?;
        let cmp = convert_value_operand(cctx, ctx, &cmp_opd)?;
        let new = convert_value_operand(cctx, ctx, &new_opd)?;
        let success = convert_atomic_ordering(
            &self
                .get_attr_llvm_cas_success_ordering(ctx)
                .expect("cmpxchg missing success ordering"),
        );
        let failure = convert_atomic_ordering(
            &self
                .get_attr_llvm_cas_failure_ordering(ctx)
                .expect("cmpxchg missing failure ordering"),
        );
        let scope = self
            .get_attr_llvm_cas_syncscope(ctx)
            .map(|s| String::from((*s).clone()))
            .unwrap_or_default();
        let ssid = llvm_get_sync_scope_id(llvm_ctx, &scope);
        Ok(llvm_build_atomic_cmpxchg(
            &cctx.builder,
            ptr,
            cmp,
            new,
            success,
            failure,
            ssid,
        ))
    }
}

#[op_interface_impl]
impl ToLLVMValue for FenceOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let ordering = convert_atomic_ordering(
            &self
                .get_attr_llvm_fence_ordering(ctx)
                .expect("fence missing ordering"),
        );
        let scope = self
            .get_attr_llvm_fence_syncscope(ctx)
            .map(|s| String::from((*s).clone()))
            .unwrap_or_default();
        let ssid = llvm_get_sync_scope_id(llvm_ctx, &scope);
        Ok(llvm_build_fence(&cctx.builder, ordering, ssid, ""))
    }
}

#[op_interface_impl]
impl ToLLVMValue for AtomicLoadOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let (ptr_opd, result_val) = {
            let op = self.get_operation().deref(ctx);
            (op.get_operand(0), op.get_result(0))
        };
        let pointee_ty = convert_type(ctx, llvm_ctx, cctx, result_val.get_type(ctx))?;
        let ptr = convert_value_operand(cctx, ctx, &ptr_opd)?;
        let load = llvm_build_load2(&cctx.builder, pointee_ty, ptr, &result_val.unique_name(ctx));
        let ordering = convert_atomic_ordering(
            &self
                .get_attr_llvm_ld_ordering(ctx)
                .expect("atomic load missing ordering"),
        );
        llvm_set_ordering(load, ordering);
        let scope = self
            .get_attr_llvm_ld_syncscope(ctx)
            .map(|s| String::from((*s).clone()))
            .unwrap_or_default();
        llvm_set_atomic_sync_scope_id(load, llvm_get_sync_scope_id(llvm_ctx, &scope));
        if let Some(alignment) = self.alignment(ctx) {
            llvm_set_alignment(load, alignment);
        }
        Ok(load)
    }
}

#[op_interface_impl]
impl ToLLVMValue for AtomicStoreOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let (val_opd, ptr_opd) = {
            let op = self.get_operation().deref(ctx);
            (op.get_operand(0), op.get_operand(1))
        };
        let value = convert_value_operand(cctx, ctx, &val_opd)?;
        let ptr = convert_value_operand(cctx, ctx, &ptr_opd)?;
        let store = llvm_build_store(&cctx.builder, value, ptr);
        let ordering = convert_atomic_ordering(
            &self
                .get_attr_llvm_st_ordering(ctx)
                .expect("atomic store missing ordering"),
        );
        llvm_set_ordering(store, ordering);
        let scope = self
            .get_attr_llvm_st_syncscope(ctx)
            .map(|s| String::from((*s).clone()))
            .unwrap_or_default();
        llvm_set_atomic_sync_scope_id(store, llvm_get_sync_scope_id(llvm_ctx, &scope));
        if let Some(alignment) = self.alignment(ctx) {
            llvm_set_alignment(store, alignment);
        }
        Ok(store)
    }
}

#[op_interface_impl]
impl ToLLVMValue for InlineAsmOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let (arg_opds, result_val) = {
            let op = self.get_operation().deref(ctx);
            let n = op.get_num_operands();
            let args: Vec<Value> = (0..n).map(|i| op.get_operand(i)).collect();
            (args, op.get_result(0))
        };
        let args: Vec<LLVMValue> = arg_opds
            .iter()
            .map(|v| convert_value_operand(cctx, ctx, v))
            .collect::<Result<_>>()?;
        let result_ty = result_val.get_type(ctx);
        let result_llvm_ty = convert_type(ctx, llvm_ctx, cctx, result_ty)?;
        let arg_types: Vec<LLVMType> = args.iter().map(|a| llvm_type_of(*a)).collect();
        let fn_ty = llvm_function_type(result_llvm_ty, &arg_types, false);
        let asm = String::from(
            (*self
                .get_attr_inline_asm_template(ctx)
                .expect("inline asm missing template"))
            .clone(),
        );
        let constraints = String::from(
            (*self
                .get_attr_inline_asm_constraints(ctx)
                .expect("inline asm missing constraints"))
            .clone(),
        );
        // `has_side_effects` is set unconditionally: this op does not model a
        // side-effects flag, and side-effecting asm is the safe default.
        // NOTE: the op's `inline_asm_convergent` attribute is not applied here.
        // `convergent` is an LLVM call-site attribute (not part of the inline-asm
        // value), so converting to LLVM IR drops the convergent flag.
        let asm_val = llvm_get_inline_asm(
            fn_ty,
            &asm,
            &constraints,
            true,
            false,
            LLVMInlineAsmDialect::LLVMInlineAsmDialectATT,
            false,
        );
        let name = if result_ty.deref(ctx).is::<VoidType>() {
            String::new()
        } else {
            result_val.unique_name(ctx).to_string()
        };
        Ok(llvm_build_call2(
            &cctx.builder,
            fn_ty,
            asm_val,
            &args,
            &name,
        ))
    }
}

#[op_interface_impl]
impl ToLLVMValue for ICmpOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let predicate = convert_ipredicate(self.predicate(ctx));
        let lhs = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let rhs = convert_value_operand(cctx, ctx, &op.get_operand(1))?;
        let icmp_op = llvm_build_icmp(
            &cctx.builder,
            predicate,
            lhs,
            rhs,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(icmp_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for ReturnOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let ret_op = if let Some(retval) = self.retval(ctx) {
            let retval = convert_value_operand(cctx, ctx, &retval)?;
            llvm_build_ret(&cctx.builder, retval)
        } else {
            llvm_build_ret_void(&cctx.builder)
        };
        Ok(ret_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for UnreachableOp {
    fn convert(
        &self,
        _ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        Ok(llvm_build_unreachable(&cctx.builder))
    }
}

#[op_interface_impl]
impl ToLLVMValue for ConstantOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        <Self as ToLLVMConstValue>::convert(self, ctx, llvm_ctx, cctx)
    }
}

#[op_interface_impl]
impl ToLLVMValue for ZeroOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        <Self as ToLLVMConstValue>::convert(self, ctx, llvm_ctx, cctx)
    }
}

#[op_interface_impl]
impl ToLLVMValue for IntToPtrOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let inttoptr_op = llvm_build_int_to_ptr(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(inttoptr_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for PtrToIntOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let ptrtoint_op = llvm_build_ptr_to_int(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(ptrtoint_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for UndefOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        <Self as ToLLVMConstValue>::convert(self, ctx, llvm_ctx, cctx)
    }
}

#[op_interface_impl]
impl ToLLVMValue for PoisonOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        <Self as ToLLVMConstValue>::convert(self, ctx, llvm_ctx, cctx)
    }
}

#[op_interface_impl]
impl ToLLVMValue for AddressOfOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        <Self as ToLLVMConstValue>::convert(self, ctx, llvm_ctx, cctx)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FreezeOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let freeze_op =
            llvm_build_freeze(&cctx.builder, arg, &self.get_result(ctx).unique_name(ctx));
        Ok(freeze_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for CallOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let args: Vec<_> = self
            .args(ctx)
            .into_iter()
            .map(|v| convert_value_operand(cctx, ctx, &v))
            .collect::<Result<_>>()?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.callee_type(ctx))?;
        let res = self.get_result(ctx);
        let name = if res.get_type(ctx).deref(ctx).is::<VoidType>() {
            ""
        } else {
            &res.unique_name(ctx)
        };
        let callee = match self.callee(ctx) {
            CallOpCallable::Direct(callee_sym) => {
                *cctx.function_map.get(&callee_sym).ok_or_else(|| {
                    input_error_noloc!(ToLLVMErr::UndefinedValue(callee_sym.to_string()))
                })?
            }
            CallOpCallable::Indirect(callee) => convert_value_operand(cctx, ctx, &callee)?,
        };
        let call_val = llvm_build_call2(&cctx.builder, ty, callee, &args, name);
        if let Some(fmf) = self.get_attr_llvm_call_fastmath_flags(ctx)
            && llvm_can_value_use_fast_math_flags(call_val)
        {
            llvm_set_fast_math_flags(call_val, (*fmf).into());
        }
        Ok(call_val)
    }
}

#[op_interface_impl]
impl ToLLVMValue for CallIntrinsicOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let args: Vec<_> = (0..op.get_num_operands())
            .map(|i| convert_value_operand(cctx, ctx, &op.get_operand(i)))
            .collect::<Result<_>>()?;
        let fn_ty = convert_type(
            ctx,
            llvm_ctx,
            cctx,
            self.get_attr_llvm_intrinsic_type(ctx)
                .unwrap()
                .get_type(ctx),
        )?;

        let intrinsic_name = <StringAttr as Into<String>>::into(
            self.get_attr_llvm_intrinsic_name(ctx)
                .expect("Intrinsic call does not name the intrinsic to be called")
                .clone(),
        );

        let _intrinsic_id = llvm_lookup_intrinsic_id(&intrinsic_name).ok_or_else(|| {
            input_error_noloc!(ToLLVMErr::UndefinedValue(intrinsic_name.to_string()))
        })?;

        // We just use llvm_add_function instead of llvm_get_intrinsic_declaration here
        // because the latter requires that (and I quote from Intrinsics.h::getOrInsertDeclaration):
        //   "For a declaration of an overloaded intrinsic, Tys must provide exactly one
        //    type for each overloaded type in the intrinsic."
        // I don't know how to determine that from just the name and argument types.
        let intrinsic_fn = llvm_get_named_function(cctx.cur_llvm_module, &intrinsic_name)
            .unwrap_or_else(|| llvm_add_function(cctx.cur_llvm_module, &intrinsic_name, fn_ty));

        let res = self.get_result(ctx);
        let name = if res.get_type(ctx).deref(ctx).is::<VoidType>() {
            ""
        } else {
            &res.unique_name(ctx)
        };

        let intrinsic_op = llvm_build_call2(&cctx.builder, fn_ty, intrinsic_fn, &args, name);

        if let Some(fmf) = self.get_attr_llvm_intrinsic_fastmath_flags(ctx)
            && llvm_can_value_use_fast_math_flags(intrinsic_op)
        {
            llvm_set_fast_math_flags(intrinsic_op, (*fmf).into());
        }

        Ok(intrinsic_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for SExtOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let sext_op = llvm_build_sext(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(sext_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for ZExtOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let zext_op = llvm_build_zext(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        // The built value may not even be an instruction, but a folded constant.
        if llvm_is_a::instruction(zext_op) {
            let nneg = self.nneg(ctx);
            llvm_set_nneg(zext_op, nneg);
        }
        Ok(zext_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for TruncOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let trunc_op = llvm_build_trunc(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(trunc_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for GetElementPtrOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let indices = self
            .indices(ctx)
            .iter()
            .map(|v| match v {
                crate::ops::GepIndex::Constant(c) => Ok(llvm_const_int(
                    llvm_int_type_in_context(llvm_ctx, 32),
                    Into::<u64>::into(*c),
                    false,
                )),
                crate::ops::GepIndex::Value(value) => convert_value_operand(cctx, ctx, value),
            })
            .collect::<Result<Vec<_>>>()?;

        let base = convert_value_operand(cctx, ctx, &self.get_operand_src_ptr(ctx))?;

        let src_elem_type = convert_type(ctx, llvm_ctx, cctx, self.src_elem_type(ctx))?;
        let gep_op = llvm_build_gep2(
            &cctx.builder,
            src_elem_type,
            base,
            &indices,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(gep_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for InsertValueOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let base = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let value = convert_value_operand(cctx, ctx, &op.get_operand(1))?;
        let indices = self.indices(ctx);
        if indices.len() != 1 {
            return input_err!(op.loc(), ToLLVMErr::InsertExtractValueIndices);
        }
        let insert_op = llvm_build_insert_value(
            &cctx.builder,
            base,
            value,
            indices[0],
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(insert_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for ExtractValueOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let base = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let indices = self.indices(ctx);
        if indices.len() != 1 {
            return input_err!(op.loc(), ToLLVMErr::InsertExtractValueIndices);
        }
        let extract_op = llvm_build_extract_value(
            &cctx.builder,
            base,
            indices[0],
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(extract_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for InsertElementOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let base = convert_value_operand(cctx, ctx, &self.get_operand_vector(ctx))?;
        let value = convert_value_operand(cctx, ctx, &self.get_operand_element(ctx))?;
        let index = convert_value_operand(cctx, ctx, &self.get_operand_index(ctx))?;
        let insert_op = llvm_build_insert_element(
            &cctx.builder,
            base,
            value,
            index,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(insert_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for ExtractElementOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let base = convert_value_operand(cctx, ctx, &self.get_operand_vector(ctx))?;
        let index = convert_value_operand(cctx, ctx, &self.get_operand_index(ctx))?;
        let extract_op = llvm_build_extract_element(
            &cctx.builder,
            base,
            index,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(extract_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for ShuffleVectorOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let mask = &self
            .get_attr_llvm_shuffle_vector_mask(ctx)
            .expect("ShuffleVectorOp missing mask attribute")
            .0;
        let op = self.get_operation().deref(ctx);
        let vec1 = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let vec2 = convert_value_operand(cctx, ctx, &op.get_operand(1))?;
        let int_ty = llvm_int_type_in_context(llvm_ctx, 32);

        let mask = mask
            .iter()
            .map(|&i| llvm_const_int(int_ty, i as u64, true))
            .collect::<Vec<LLVMValue>>();
        let mask = llvm_const_vector(&mask);

        let shuffle_op = llvm_build_shuffle_vector(
            &cctx.builder,
            vec1,
            vec2,
            mask,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(shuffle_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for SelectOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let cond = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let true_val = convert_value_operand(cctx, ctx, &op.get_operand(1))?;
        let false_val = convert_value_operand(cctx, ctx, &op.get_operand(2))?;
        let select_op = llvm_build_select(
            &cctx.builder,
            cond,
            true_val,
            false_val,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(select_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FAddOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let (lhs, rhs) = (op.get_operand(0), op.get_operand(1));
        let lhs = convert_value_operand(cctx, ctx, &lhs)?;
        let rhs = convert_value_operand(cctx, ctx, &rhs)?;
        let inst = llvm_build_fadd(
            &cctx.builder,
            lhs,
            rhs,
            &self.get_result(ctx).unique_name(ctx),
        );
        // The built value may not even be an instruction, but a folded constant.
        if llvm_can_value_use_fast_math_flags(inst) {
            let fastmath = self.fast_math_flags(ctx);
            llvm_set_fast_math_flags(inst, fastmath.into());
        }
        Ok(inst)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FSubOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let (lhs, rhs) = (op.get_operand(0), op.get_operand(1));
        let lhs = convert_value_operand(cctx, ctx, &lhs)?;
        let rhs = convert_value_operand(cctx, ctx, &rhs)?;
        let inst = llvm_build_fsub(
            &cctx.builder,
            lhs,
            rhs,
            &self.get_result(ctx).unique_name(ctx),
        );
        // The built value may not even be an instruction, but a folded constant.
        if llvm_can_value_use_fast_math_flags(inst) {
            let fastmath = self.fast_math_flags(ctx);
            llvm_set_fast_math_flags(inst, fastmath.into());
        }
        Ok(inst)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FMulOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let (lhs, rhs) = (op.get_operand(0), op.get_operand(1));
        let lhs = convert_value_operand(cctx, ctx, &lhs)?;
        let rhs = convert_value_operand(cctx, ctx, &rhs)?;
        let inst = llvm_build_fmul(
            &cctx.builder,
            lhs,
            rhs,
            &self.get_result(ctx).unique_name(ctx),
        );
        // The built value may not even be an instruction, but a folded constant.
        if llvm_can_value_use_fast_math_flags(inst) {
            let fastmath = self.fast_math_flags(ctx);
            llvm_set_fast_math_flags(inst, fastmath.into());
        }
        Ok(inst)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FDivOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let (lhs, rhs) = (op.get_operand(0), op.get_operand(1));
        let lhs = convert_value_operand(cctx, ctx, &lhs)?;
        let rhs = convert_value_operand(cctx, ctx, &rhs)?;
        let inst = llvm_build_fdiv(
            &cctx.builder,
            lhs,
            rhs,
            &self.get_result(ctx).unique_name(ctx),
        );
        // The built value may not even be an instruction, but a folded constant.
        if llvm_can_value_use_fast_math_flags(inst) {
            let fastmath = self.fast_math_flags(ctx);
            llvm_set_fast_math_flags(inst, fastmath.into());
        }
        Ok(inst)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FRemOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let (lhs, rhs) = (op.get_operand(0), op.get_operand(1));
        let lhs = convert_value_operand(cctx, ctx, &lhs)?;
        let rhs = convert_value_operand(cctx, ctx, &rhs)?;
        let inst = llvm_build_frem(
            &cctx.builder,
            lhs,
            rhs,
            &self.get_result(ctx).unique_name(ctx),
        );
        // The built value may not even be an instruction, but a folded constant.
        if llvm_can_value_use_fast_math_flags(inst) {
            let fastmath = self.fast_math_flags(ctx);
            llvm_set_fast_math_flags(inst, fastmath.into());
        }
        Ok(inst)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FCmpOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let predicate = convert_fpredicate(self.predicate(ctx));
        let lhs = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let rhs = convert_value_operand(cctx, ctx, &op.get_operand(1))?;
        let fcmp_op = llvm_build_fcmp(
            &cctx.builder,
            predicate,
            lhs,
            rhs,
            &self.get_result(ctx).unique_name(ctx),
        );
        // The built value may not even be an instruction, but a folded constant.
        if llvm_can_value_use_fast_math_flags(fcmp_op) {
            let fastmath = self.fast_math_flags(ctx);
            llvm_set_fast_math_flags(fcmp_op, fastmath.into());
        }
        Ok(fcmp_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FPExtOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let fpext_op = llvm_build_fpext(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        // The built value may not even be an instruction, but a folded constant.
        if llvm_can_value_use_fast_math_flags(fpext_op) {
            let fastmath = self.fast_math_flags(ctx);
            llvm_set_fast_math_flags(fpext_op, fastmath.into());
        }
        Ok(fpext_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FPTruncOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let fptrunc_op = llvm_build_fptrunc(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        // The built value may not even be an instruction, but a folded constant.
        if llvm_can_value_use_fast_math_flags(fptrunc_op) {
            llvm_set_fast_math_flags(fptrunc_op, self.fast_math_flags(ctx).into());
        }
        Ok(fptrunc_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FPToSIOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let fptosi_op = llvm_build_fptosi(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(fptosi_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for SIToFPOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let sitofp_op = llvm_build_sitofp(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(sitofp_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for FPToUIOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let fptoui_op = llvm_build_fptoui(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(fptoui_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for UIToFPOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let uitofp_op = llvm_build_uitofp(
            &cctx.builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        // The built value may not even be an instruction, but a folded constant.
        if llvm_is_a::instruction(uitofp_op) {
            let nneg = self.nneg(ctx);
            llvm_set_nneg(uitofp_op, nneg);
        }
        Ok(uitofp_op)
    }
}

#[op_interface_impl]
impl ToLLVMValue for VAArgOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let opd = convert_value_operand(cctx, ctx, &op.get_operand(0))?;
        log::warn!("Generating va_arg instruction: It is poorly supported by LLVM");
        let vaarg_op = llvm_build_va_arg(
            &cctx.builder,
            opd,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        Ok(vaarg_op)
    }
}

/// Convert a pliron [BasicBlock] to [LLVMBasicBlock].
fn convert_block(
    ctx: &Context,
    llvm_ctx: &LLVMContext,
    cctx: &mut ConversionContext,
    block: Ptr<BasicBlock>,
) -> Result<()> {
    let block_llvm = cctx.block_map[&block];
    llvm_position_builder_at_end(&cctx.builder, block_llvm);

    for opr in block.deref(ctx).iter(ctx) {
        let op = Operation::get_op_dyn(opr, ctx);
        let op = op.as_ref();
        let Some(op_conv) = op_cast::<dyn ToLLVMValue>(op) else {
            let loc = op.loc(ctx);
            return input_err!(
                loc,
                ToLLVMErr::MissingOpConversion(op.get_opid().to_string())
            );
        };
        let op_llvm = op_conv.convert(ctx, llvm_ctx, cctx)?;
        let opr_ref = opr.deref(ctx);
        // LLVM instructions have at most one result.
        if opr_ref.get_num_results() == 1 {
            cctx.value_map.insert(opr_ref.get_result(0), op_llvm);
        }
    }

    Ok(())
}

/// Convert a pliron [FuncOp] to [LLVMValue]
fn convert_function(
    ctx: &Context,
    llvm_ctx: &LLVMContext,
    cctx: &mut ConversionContext,
    func_op: FuncOp,
) -> Result<LLVMValue> {
    cctx.clear_per_function_data();
    let func_llvm = cctx.function_map[&func_op.get_symbol_name(ctx)];

    if let Some(linkage) = func_op.get_attr_llvm_function_linkage(ctx) {
        let llvm_linkage: LLVMLinkage = convert_linkage(linkage.clone());
        llvm_set_linkage(func_llvm, llvm_linkage);
    }

    let f_region = func_op.get_region(ctx).expect("Function missing region");

    // Map all blocks, staring with entry.
    let mut block_iter = f_region.deref(ctx).iter(ctx);
    {
        let entry = block_iter.next().expect("Missing entry block");
        // Map entry block arguments to LLVM function arguments.
        for (arg_idx, arg) in entry.deref(ctx).arguments().enumerate() {
            cctx.value_map
                .insert(arg, llvm_get_param(func_llvm, arg_idx.try_into().unwrap()));
        }
        let llvm_entry_block = llvm_append_basic_block_in_context(
            llvm_ctx,
            func_llvm,
            &entry.deref(ctx).unique_name(ctx),
        );
        cctx.block_map.insert(entry, llvm_entry_block);
    }
    for block in block_iter {
        let llvm_block = llvm_append_basic_block_in_context(
            llvm_ctx,
            func_llvm,
            &block.deref(ctx).unique_name(ctx),
        );
        llvm_position_builder_at_end(&cctx.builder, llvm_block);
        for arg in block.deref(ctx).arguments() {
            let arg_type = convert_type(ctx, llvm_ctx, cctx, arg.get_type(ctx))?;
            let phi = llvm_build_phi(&cctx.builder, arg_type, &arg.unique_name(ctx));
            cctx.value_map.insert(arg, phi);
        }
        cctx.block_map.insert(block, llvm_block);
    }

    // Convert within every block.
    for block in topological_order(ctx, &f_region) {
        convert_block(ctx, llvm_ctx, cctx, block)?;
    }

    Ok(func_llvm)
}

#[op_interface]
trait ToLLVMConstValue {
    /// Convert from pliron [Op] to a constant [LLVMValue].
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue>;

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[op_interface_impl]
impl ToLLVMConstValue for ConstantOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let value = self.get_value(ctx);
        if let Some(int_val) = value.downcast_ref::<IntegerAttr>() {
            let int_ty = int_val.get_type();
            let int_ty_llvm = convert_type(ctx, llvm_ctx, cctx, int_ty.into())?;
            let ap_int_val: APInt = int_val.clone().into();
            let const_val = llvm_const_int(int_ty_llvm, ap_int_val.to_u64(), false);
            Ok(const_val)
        } else if let Some(float_val) = attr_cast::<dyn FloatAttrToFP64>(&*value) {
            let float_ty = float_val.get_type(ctx);
            let float_ty_llvm = convert_type(ctx, llvm_ctx, cctx, float_ty)?;
            let const_val = llvm_const_real(float_ty_llvm, float_val.to_fp64());
            Ok(const_val)
        } else {
            input_err!(op.loc(), ToLLVMErr::ConstOpNotIntOrFloat)
        }
    }
}

#[op_interface_impl]
impl ToLLVMConstValue for UndefOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        Ok(llvm_get_undef(ty))
    }
}

#[op_interface_impl]
impl ToLLVMConstValue for PoisonOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        Ok(llvm_get_poison(ty))
    }
}

#[op_interface_impl]
impl ToLLVMConstValue for ZeroOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;
        let zero_val = llvm_const_null(ty);
        Ok(zero_val)
    }
}

#[op_interface_impl]
impl ToLLVMConstValue for AddressOfOp {
    fn convert(
        &self,
        ctx: &Context,
        _llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let sym = self.get_global_name(ctx);
        cctx.globals_map
            .get(&sym)
            .or_else(|| cctx.function_map.get(&sym))
            .cloned()
            .ok_or_else(|| input_error_noloc!(ToLLVMErr::CannotEvaluateToConst))
    }
}
#[op_interface_impl]
impl ToLLVMConstValue for InsertValueOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let base = convert_to_llvm_const(ctx, cctx, llvm_ctx, op.get_operand(0))?;
        let value = convert_to_llvm_const(ctx, cctx, llvm_ctx, op.get_operand(1))?;
        let indices = self.indices(ctx);
        if indices.len() != 1 {
            return input_err!(op.loc(), ToLLVMErr::InsertExtractValueIndices);
        }

        // LLVM's builder tries to fold this, so we rely on that.
        let insert_op = llvm_build_insert_value(
            &cctx.scratch_builder,
            base,
            value,
            indices[0],
            &self.get_result(ctx).unique_name(ctx),
        );
        if !llvm_is_a::constant(insert_op) {
            return input_err!(op.loc(), ToLLVMErr::CannotEvaluateToConst);
        }
        Ok(insert_op)
    }
}

#[op_interface_impl]
impl ToLLVMConstValue for InsertElementOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let base = convert_to_llvm_const(ctx, cctx, llvm_ctx, op.get_operand(0))?;
        let value = convert_to_llvm_const(ctx, cctx, llvm_ctx, op.get_operand(1))?;
        let index = self.get_operand_index(ctx);
        let index = convert_to_llvm_const(ctx, cctx, llvm_ctx, index)?;

        // LLVM's builder tries to fold this, so we rely on that.
        let insert_op = llvm_build_insert_element(
            &cctx.scratch_builder,
            base,
            value,
            index,
            &self.get_result(ctx).unique_name(ctx),
        );
        if !llvm_is_a::constant(insert_op) {
            return input_err!(op.loc(), ToLLVMErr::CannotEvaluateToConst);
        }
        Ok(insert_op)
    }
}

#[op_interface_impl]
impl ToLLVMConstValue for TruncOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_to_llvm_const(ctx, cctx, llvm_ctx, op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;

        // LLVM's builder tries to fold this, so we rely on that.
        let trunc_op = llvm_build_trunc(
            &cctx.scratch_builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        if !llvm_is_a::constant(trunc_op) {
            return input_err!(op.loc(), ToLLVMErr::CannotEvaluateToConst);
        }
        Ok(trunc_op)
    }
}

#[op_interface_impl]
impl ToLLVMConstValue for SubOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let lhs = convert_to_llvm_const(ctx, cctx, llvm_ctx, op.get_operand(0))?;
        let rhs = convert_to_llvm_const(ctx, cctx, llvm_ctx, op.get_operand(1))?;

        // LLVM's builder tries to fold this, so we rely on that.
        let sub_op = llvm_build_sub(
            &cctx.scratch_builder,
            lhs,
            rhs,
            &self.get_result(ctx).unique_name(ctx),
        );
        if !llvm_is_a::constant(sub_op) {
            return input_err!(op.loc(), ToLLVMErr::CannotEvaluateToConst);
        }
        Ok(sub_op)
    }
}

#[op_interface_impl]
impl ToLLVMConstValue for PtrToIntOp {
    fn convert(
        &self,
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
    ) -> Result<LLVMValue> {
        let op = self.get_operation().deref(ctx);
        let arg = convert_to_llvm_const(ctx, cctx, llvm_ctx, op.get_operand(0))?;
        let ty = convert_type(ctx, llvm_ctx, cctx, self.result_type(ctx))?;

        // LLVM's builder tries to fold this, so we rely on that.
        let ptoi_op = llvm_build_ptr_to_int(
            &cctx.scratch_builder,
            arg,
            ty,
            &self.get_result(ctx).unique_name(ctx),
        );
        if !llvm_is_a::constant(ptoi_op) {
            return input_err!(op.loc(), ToLLVMErr::CannotEvaluateToConst);
        }
        Ok(ptoi_op)
    }
}

fn convert_to_llvm_const(
    ctx: &Context,
    cctx: &mut ConversionContext,
    llvm_ctx: &LLVMContext,
    value: Value,
) -> Result<LLVMValue> {
    match value.defining_entity() {
        DefiningEntity::Op(op) => {
            let op = Operation::get_op_dyn(op, ctx);
            if let Some(const_trans) = op_cast::<dyn ToLLVMConstValue>(op.as_ref()) {
                const_trans.convert(ctx, llvm_ctx, cctx)
            } else {
                input_err!(value.loc(ctx), ToLLVMErr::CannotEvaluateToConst)
            }
        }
        DefiningEntity::Block(_) => {
            input_err!(value.loc(ctx), ToLLVMErr::CannotEvaluateToConst)
        }
    }
}

fn convert_global_initializer(
    ctx: &Context,
    llvm_ctx: &LLVMContext,
    cctx: &mut ConversionContext,
    global_op: GlobalOp,
) -> Result<Option<LLVMValue>> {
    if let Some(_initializer) = global_op.get_initializer_value(ctx) {
        todo!()
    }

    if let Some(init_block) = global_op.get_initializer_block(ctx) {
        let ret =
            Operation::get_op::<ReturnOp>(init_block.deref(ctx).get_terminator(ctx).unwrap(), ctx);
        let ret = ret.ok_or_else(|| {
            input_error!(
                global_op.loc(ctx),
                ToLLVMErr::GlobalOpInitializerRegionBadReturn
            )
        })?;
        let Some(ret_val) = ret.retval(ctx) else {
            return input_err!(
                global_op.loc(ctx),
                ToLLVMErr::GlobalOpInitializerRegionBadReturn
            );
        };
        let initializer_val = convert_to_llvm_const(ctx, cctx, llvm_ctx, ret_val)?;
        return Ok(Some(initializer_val));
    }

    Ok(None)
}

/// Convert pliron [ModuleOp] to [LLVMModule].
pub fn convert_module(
    ctx: &Context,
    llvm_ctx: &LLVMContext,
    module: ModuleOp,
) -> Result<LLVMModule> {
    let mod_name = module.get_symbol_name(ctx);
    let llvm_module = LLVMModule::new(&mod_name, llvm_ctx);
    let cctx = &mut ConversionContext::new(llvm_ctx, &llvm_module);

    // Setup the scratch builder for evaluating constants.
    // `scratch_module` is freed at the end of this function, when it exits the scope.
    let scratch_module = LLVMModule::new("__pliron_scratch_module", llvm_ctx);
    let scratch_function = llvm_add_function(
        &scratch_module,
        "scratch",
        llvm_function_type(llvm_void_type_in_context(llvm_ctx), &[], false),
    );
    let scratch_function_entry =
        llvm_append_basic_block_in_context(llvm_ctx, scratch_function, "entry");
    llvm_position_builder_at_end(&cctx.scratch_builder, scratch_function_entry);

    // Create new functions and map them.
    for op in module.get_body(ctx, 0).deref(ctx).iter(ctx) {
        if let Some(func_op) = Operation::get_op::<FuncOp>(op, ctx) {
            let func_ty = func_op.get_type(ctx).deref(ctx);
            let func_ty_to_llvm = type_cast::<dyn ToLLVMType>(&*func_ty).ok_or_else(|| {
                input_error_noloc!(ToLLVMErr::MissingTypeConversion(
                    func_ty.disp(ctx).to_string()
                ))
            })?;
            let fn_ty_llvm = func_ty_to_llvm.convert(ctx, llvm_ctx, cctx)?;
            let name = func_op.get_symbol_name(ctx);
            let llvm_name = func_op.llvm_symbol_name(ctx).unwrap_or(name.clone().into());
            let func_llvm = llvm_add_function(&llvm_module, &llvm_name, fn_ty_llvm);
            cctx.function_map.insert(name, func_llvm);
        }
        if let Some(global_op) = Operation::get_op::<GlobalOp>(op, ctx) {
            let global_ty = global_op.get_type(ctx);
            let global_ty_llvm = convert_type(ctx, llvm_ctx, cctx, global_ty)?;
            let global_name = global_op.get_symbol_name(ctx);
            let llvm_global_name = global_op
                .llvm_symbol_name(ctx)
                .unwrap_or(global_name.clone().into());
            let global_addr_space = global_op.address_space(ctx);
            let global_llvm = llvm_add_global_in_address_space(
                &llvm_module,
                global_ty_llvm,
                &llvm_global_name,
                global_addr_space,
            );
            cctx.globals_map.insert(global_name, global_llvm);
        }
    }

    for op in module.get_body(ctx, 0).deref(ctx).iter(ctx) {
        if let Some(func_op) = Operation::get_op::<FuncOp>(op, ctx)
            && !func_op.is_declaration(ctx)
        {
            convert_function(ctx, llvm_ctx, cctx, func_op)?;
        }
        if let Some(global_op) = Operation::get_op::<GlobalOp>(op, ctx) {
            let global_name = global_op.get_symbol_name(ctx);
            let global_llvm = cctx.globals_map[&global_name];
            if !global_op.is_declaration(ctx)
                && let Some(initializer) =
                    convert_global_initializer(ctx, llvm_ctx, cctx, global_op)?
            {
                llvm_set_initializer(global_llvm, initializer);
            }
            if let Some(linkage) = global_op.get_attr_llvm_global_linkage(ctx) {
                let llvm_linkage: LLVMLinkage = convert_linkage(linkage.clone());
                llvm_set_linkage(global_llvm, llvm_linkage);
            }
            if let Some(alignment) = global_op.alignment(ctx) {
                llvm_set_alignment(global_llvm, alignment);
            }
        }
    }

    Ok(llvm_module)
}
