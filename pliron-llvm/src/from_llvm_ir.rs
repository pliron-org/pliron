//! Translate from LLVM-IR to pliron's LLVM dialect

use std::num::NonZero;

use llvm_sys::{
    LLVMAtomicOrdering, LLVMAtomicRMWBinOp, LLVMIntPredicate, LLVMLinkage, LLVMOpcode,
    LLVMRealPredicate, LLVMTypeKind, LLVMValueKind,
};
use pliron::{
    attribute::AttrObj,
    basic_block::BasicBlock,
    builtin::{
        attributes::{FPDoubleAttr, FPSingleAttr, IntegerAttr},
        op_interfaces::{
            AtMostOneRegionInterface, CallOpCallable, OneResultInterface,
            SingleBlockRegionInterface,
        },
        ops::ModuleOp,
        type_interfaces::FloatTypeInterface,
        types::{FP16Type, FP32Type, FP64Type, IntegerType, Signedness},
    },
    context::{Context, Ptr},
    debug_info,
    derive::{type_interface, type_interface_impl},
    identifier::{self, Identifier},
    input_err_noloc, input_error_noloc,
    irbuild::{
        inserter::{IRInserter, Inserter},
        listener::DummyListener,
    },
    linked_list::ContainsLinkedList,
    op::Op,
    operation::Operation,
    result::Result,
    r#type::{Type, TypeHandle, TypedHandle, type_cast},
    utils::apint::APInt,
    value::Value,
};
use rustc_hash::{FxHashMap, FxHashSet};
use thiserror::Error;

use crate::{
    attributes::{
        AtomicOrderingAttr, AtomicRmwKindAttr, FCmpPredicateAttr, FastmathFlagsAttr,
        ICmpPredicateAttr, IntegerOverflowFlagsAttr, LinkageAttr,
    },
    llvm_sys::core::{
        LLVMBasicBlock, LLVMModule, LLVMType, LLVMValue, basic_block_iter, function_iter,
        global_iter, incoming_iter, instruction_iter, llvm_can_value_use_fast_math_flags,
        llvm_const_int_get_zext_value, llvm_const_real_get_double, llvm_count_struct_element_types,
        llvm_get_aggregate_element, llvm_get_alignment, llvm_get_allocated_type,
        llvm_get_array_length2, llvm_get_atomic_rmw_bin_op, llvm_get_atomic_sync_scope_id,
        llvm_get_basic_block_name, llvm_get_basic_block_terminator, llvm_get_called_function_type,
        llvm_get_called_value, llvm_get_cmpxchg_failure_ordering,
        llvm_get_cmpxchg_success_ordering, llvm_get_const_opcode, llvm_get_element_type,
        llvm_get_fast_math_flags, llvm_get_fcmp_predicate, llvm_get_gep_source_element_type,
        llvm_get_icmp_predicate, llvm_get_indices, llvm_get_initializer,
        llvm_get_inline_asm_asm_string, llvm_get_inline_asm_constraint_string,
        llvm_get_instruction_opcode, llvm_get_instruction_parent, llvm_get_int_type_width,
        llvm_get_linkage, llvm_get_mask_value, llvm_get_module_identifier, llvm_get_nneg,
        llvm_get_nsw, llvm_get_num_arg_operands, llvm_get_num_mask_elements, llvm_get_num_operands,
        llvm_get_nuw, llvm_get_operand, llvm_get_ordering, llvm_get_param_types,
        llvm_get_pointer_address_space, llvm_get_return_type, llvm_get_struct_element_types,
        llvm_get_struct_name, llvm_get_switch_case_value, llvm_get_type_kind, llvm_get_value_kind,
        llvm_get_value_name, llvm_get_vector_size, llvm_global_get_value_type, llvm_is_a,
        llvm_is_declaration, llvm_is_function_type_var_arg, llvm_is_opaque_struct,
        llvm_lookup_intrinsic_id, llvm_print_value_to_string, llvm_type_of,
        llvm_value_as_basic_block, llvm_value_is_basic_block, param_iter,
    },
    op_interfaces::{
        AlignableOpInterface, BinArithOp, CastOpInterface, CastOpWithNNegInterface, FastMathFlags,
        FloatBinArithOpWithFastMathFlags, IntBinArithOpWithOverflowFlag, LlvmSymbolName,
    },
    ops::{
        AShrOp, AddOp, AddrSpaceCastOp, AddressOfOp, AllocaOp, AndOp, AtomicCmpxchgOp,
        AtomicLoadOp, AtomicRmwOp, AtomicStoreOp, BitcastOp, BrOp, CallIntrinsicOp, CallOp,
        CondBrOp, ConstantOp, ExtractElementOp, ExtractValueOp, FAddOp, FCmpOp, FDivOp, FMulOp,
        FNegOp, FPExtOp, FPToSIOp, FPToUIOp, FPTruncOp, FRemOp, FSubOp, FenceOp, FreezeOp, FuncOp,
        GepIndex, GetElementPtrOp, GlobalOp, ICmpOp, InlineAsmOp, InsertElementOp, InsertValueOp,
        IntToPtrOp, LShrOp, LoadOp, MulOp, OrOp, PoisonOp, PtrToIntOp, ReturnOp, SDivOp, SExtOp,
        SIToFPOp, SRemOp, SelectOp, ShlOp, ShuffleVectorOp, StoreOp, SubOp, SwitchCase, SwitchOp,
        TruncOp, UDivOp, UIToFPOp, URemOp, UndefOp, UnreachableOp, VAArgOp, XorOp, ZExtOp, ZeroOp,
    },
    types::{
        ArrayType, FuncType, PointerType, StructErr, StructType, VectorType, VectorTypeKind,
        VoidType,
    },
};

/// Given a floating point type and an f64 value, get an equivalent attribute.
/// LLVM's C-API pretty much restricts us to f64 for floating point constants.
#[type_interface]
trait FloatAttrBuilder: FloatTypeInterface {
    fn value_from_f64(&self, val: f64) -> AttrObj;
    fn verify(_attr: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[type_interface_impl]
impl FloatAttrBuilder for FP32Type {
    fn value_from_f64(&self, val: f64) -> AttrObj {
        FPSingleAttr::from(val as f32).into()
    }
}

#[type_interface_impl]
impl FloatAttrBuilder for FP64Type {
    fn value_from_f64(&self, val: f64) -> AttrObj {
        FPDoubleAttr::from(val).into()
    }
}

fn convert_type(
    ctx: &mut Context,
    cctx: &mut ConversionContext,
    ty: LLVMType,
) -> Result<TypeHandle> {
    if let Some(cached) = cctx.type_cache.get(&ty) {
        return Ok(*cached);
    }
    let kind = llvm_get_type_kind(ty);
    let converted_ty: Result<TypeHandle> = match kind {
        LLVMTypeKind::LLVMArrayTypeKind => {
            let (element_ty, len) = (llvm_get_element_type(ty), llvm_get_array_length2(ty));
            let elem = convert_type(ctx, cctx, element_ty)?;
            Ok(ArrayType::get(ctx, elem, len).into())
        }
        LLVMTypeKind::LLVMFunctionTypeKind => {
            let return_type = convert_type(ctx, cctx, llvm_get_return_type(ty))?;
            let param_types = llvm_get_param_types(ty)
                .into_iter()
                .map(|ty| convert_type(ctx, cctx, ty))
                .collect::<Result<_>>()?;
            let is_var_arg = llvm_is_function_type_var_arg(ty);
            Ok(FuncType::get(ctx, return_type, param_types, is_var_arg).into())
        }
        LLVMTypeKind::LLVMIntegerTypeKind => {
            let bit_width = llvm_get_int_type_width(ty);
            Ok(IntegerType::get(ctx, bit_width, Signedness::Signless).into())
        }
        LLVMTypeKind::LLVMPointerTypeKind => {
            Ok(PointerType::get(ctx, llvm_get_pointer_address_space(ty)).into())
        }
        LLVMTypeKind::LLVMStructTypeKind => {
            let name_opt: Option<Identifier> =
                llvm_get_struct_name(ty).map(|str| cctx.id_legaliser.legalise(&str));
            if llvm_is_opaque_struct(ty) {
                // Opaque structs must be named.
                let Some(name) = name_opt else {
                    return input_err_noloc!(StructErr::OpaqueAndAnonymousErr);
                };
                Ok(StructType::get_named(ctx, name, None)?.into())
            } else {
                let field_types = llvm_get_struct_element_types(ty)
                    .into_iter()
                    .map(|ty| convert_type(ctx, cctx, ty))
                    .collect::<Result<_>>()?;
                if let Some(name) = name_opt {
                    Ok(StructType::get_named(ctx, name, Some(field_types))?.into())
                } else {
                    Ok(StructType::get_unnamed(ctx, field_types).into())
                }
            }
        }
        LLVMTypeKind::LLVMVoidTypeKind => Ok(VoidType::get(ctx).into()),
        LLVMTypeKind::LLVMFloatTypeKind => Ok(FP32Type::get(ctx).into()),
        LLVMTypeKind::LLVMDoubleTypeKind => Ok(FP64Type::get(ctx).into()),
        LLVMTypeKind::LLVMVectorTypeKind | LLVMTypeKind::LLVMScalableVectorTypeKind => {
            let element_ty = llvm_get_element_type(ty);
            let elem_ty = convert_type(ctx, cctx, element_ty)?;
            let num_elements = llvm_get_vector_size(ty);
            let kind = if matches!(kind, LLVMTypeKind::LLVMScalableVectorTypeKind) {
                VectorTypeKind::Scalable
            } else {
                VectorTypeKind::Fixed
            };
            Ok(VectorType::get(ctx, elem_ty, num_elements, kind).into())
        }
        LLVMTypeKind::LLVMHalfTypeKind => Ok(FP16Type::get(ctx).into()),
        LLVMTypeKind::LLVMX86_FP80TypeKind => todo!(),
        LLVMTypeKind::LLVMFP128TypeKind => todo!(),
        LLVMTypeKind::LLVMPPC_FP128TypeKind => todo!(),
        LLVMTypeKind::LLVMLabelTypeKind => todo!(),
        LLVMTypeKind::LLVMMetadataTypeKind => todo!(),
        LLVMTypeKind::LLVMTokenTypeKind => todo!(),
        LLVMTypeKind::LLVMBFloatTypeKind => todo!(),
        LLVMTypeKind::LLVMX86_AMXTypeKind => todo!(),
        LLVMTypeKind::LLVMTargetExtTypeKind => todo!(),
    };

    let converted_ty = converted_ty?;
    cctx.type_cache.insert(ty, converted_ty);
    Ok(converted_ty)
}

pub fn convert_ipredicate(ipred: LLVMIntPredicate) -> ICmpPredicateAttr {
    match ipred {
        LLVMIntPredicate::LLVMIntEQ => ICmpPredicateAttr::EQ,
        LLVMIntPredicate::LLVMIntNE => ICmpPredicateAttr::NE,
        LLVMIntPredicate::LLVMIntUGT => ICmpPredicateAttr::UGT,
        LLVMIntPredicate::LLVMIntUGE => ICmpPredicateAttr::UGE,
        LLVMIntPredicate::LLVMIntULT => ICmpPredicateAttr::ULT,
        LLVMIntPredicate::LLVMIntULE => ICmpPredicateAttr::ULE,
        LLVMIntPredicate::LLVMIntSGT => ICmpPredicateAttr::SGT,
        LLVMIntPredicate::LLVMIntSGE => ICmpPredicateAttr::SGE,
        LLVMIntPredicate::LLVMIntSLT => ICmpPredicateAttr::SLT,
        LLVMIntPredicate::LLVMIntSLE => ICmpPredicateAttr::SLE,
    }
}

pub fn convert_fpredicate(fpred: LLVMRealPredicate) -> FCmpPredicateAttr {
    match fpred {
        LLVMRealPredicate::LLVMRealPredicateFalse => FCmpPredicateAttr::False,
        LLVMRealPredicate::LLVMRealOEQ => FCmpPredicateAttr::OEQ,
        LLVMRealPredicate::LLVMRealOGT => FCmpPredicateAttr::OGT,
        LLVMRealPredicate::LLVMRealOGE => FCmpPredicateAttr::OGE,
        LLVMRealPredicate::LLVMRealOLT => FCmpPredicateAttr::OLT,
        LLVMRealPredicate::LLVMRealOLE => FCmpPredicateAttr::OLE,
        LLVMRealPredicate::LLVMRealONE => FCmpPredicateAttr::ONE,
        LLVMRealPredicate::LLVMRealORD => FCmpPredicateAttr::ORD,
        LLVMRealPredicate::LLVMRealUNO => FCmpPredicateAttr::UNO,
        LLVMRealPredicate::LLVMRealUEQ => FCmpPredicateAttr::UEQ,
        LLVMRealPredicate::LLVMRealUGT => FCmpPredicateAttr::UGT,
        LLVMRealPredicate::LLVMRealUGE => FCmpPredicateAttr::UGE,
        LLVMRealPredicate::LLVMRealULT => FCmpPredicateAttr::ULT,
        LLVMRealPredicate::LLVMRealULE => FCmpPredicateAttr::ULE,
        LLVMRealPredicate::LLVMRealUNE => FCmpPredicateAttr::UNE,
        LLVMRealPredicate::LLVMRealPredicateTrue => FCmpPredicateAttr::True,
    }
}

pub fn convert_linkage(linkage: LLVMLinkage) -> LinkageAttr {
    match linkage {
        LLVMLinkage::LLVMExternalLinkage => LinkageAttr::ExternalLinkage,
        LLVMLinkage::LLVMAvailableExternallyLinkage => LinkageAttr::AvailableExternallyLinkage,
        LLVMLinkage::LLVMLinkOnceAnyLinkage => LinkageAttr::LinkOnceAnyLinkage,
        LLVMLinkage::LLVMLinkOnceODRLinkage => LinkageAttr::LinkOnceODRLinkage,
        LLVMLinkage::LLVMWeakAnyLinkage => LinkageAttr::WeakAnyLinkage,
        LLVMLinkage::LLVMWeakODRLinkage => LinkageAttr::WeakODRLinkage,
        LLVMLinkage::LLVMLinkOnceODRAutoHideLinkage => LinkageAttr::LinkOnceODRAutoHideLinkage,
        LLVMLinkage::LLVMCommonLinkage => LinkageAttr::CommonLinkage,
        LLVMLinkage::LLVMAppendingLinkage => LinkageAttr::AppendingLinkage,
        LLVMLinkage::LLVMInternalLinkage => LinkageAttr::InternalLinkage,
        LLVMLinkage::LLVMPrivateLinkage => LinkageAttr::PrivateLinkage,
        LLVMLinkage::LLVMDLLImportLinkage => LinkageAttr::DLLImportLinkage,
        LLVMLinkage::LLVMDLLExportLinkage => LinkageAttr::DLLExportLinkage,
        LLVMLinkage::LLVMExternalWeakLinkage => LinkageAttr::ExternalWeakLinkage,
        LLVMLinkage::LLVMLinkerPrivateLinkage => LinkageAttr::LinkerPrivateLinkage,
        LLVMLinkage::LLVMLinkerPrivateWeakLinkage => LinkageAttr::LinkerPrivateWeakLinkage,
        LLVMLinkage::LLVMGhostLinkage => LinkageAttr::GhostLinkage,
    }
}

/// Mapping from LLVM entities to pliron entities.
#[derive(Default)]
struct ConversionContext {
    /// A map from LLVM's Values to pliron's Values.
    value_map: FxHashMap<LLVMValue, Value>,
    /// A map from LLVM's basic blocks to plirons'.
    block_map: FxHashMap<LLVMBasicBlock, Ptr<BasicBlock>>,
    /// Cache already converted types.
    type_cache: FxHashMap<LLVMType, TypeHandle>,
    /// Insertion point for constants in the entry block.
    constants_inserter: Option<IRInserter<DummyListener>>,
    /// Identifier legaliser
    id_legaliser: identifier::Legaliser,
}

impl ConversionContext {
    /// Reset the value and block maps and initialize
    /// constants inserter to the start of the entry block.
    /// Identifier::Legaliser remains unmodified.
    fn reset_for_region(&mut self, entry_block: Ptr<BasicBlock>) {
        self.constants_inserter = Some(IRInserter::new_at_block_start(entry_block));
        self.value_map.clear();
        self.block_map.clear();
    }
}

/// Get the successors of an LLVM block.
fn successors(block: LLVMBasicBlock) -> Vec<LLVMBasicBlock> {
    let Some(term) = llvm_get_basic_block_terminator(block) else {
        return vec![];
    };

    match llvm_get_instruction_opcode(term) {
        LLVMOpcode::LLVMBr => {
            if llvm_get_num_operands(term) == 1 {
                // Conditional branch
                vec![llvm_value_as_basic_block(llvm_get_operand(term, 0))]
            } else {
                assert!(llvm_get_num_operands(term) == 3);
                vec![
                    llvm_value_as_basic_block(llvm_get_operand(term, 1)),
                    llvm_value_as_basic_block(llvm_get_operand(term, 2)),
                ]
            }
        }
        LLVMOpcode::LLVMSwitch => {
            // The first two operands are the condition value and the default destination.
            // After that there are pairs of case value and destination.
            let num_cases = (llvm_get_num_operands(term) - 2) / 2;
            let mut succs = vec![llvm_value_as_basic_block(llvm_get_operand(term, 1))];
            for i in 0..num_cases {
                succs.push(llvm_value_as_basic_block(llvm_get_operand(
                    term,
                    2 + (2 * i) + 1,
                )));
            }
            succs
        }
        LLVMOpcode::LLVMRet | LLVMOpcode::LLVMUnreachable => {
            // No successors.
            vec![]
        }
        _ => {
            todo!(
                "Unsupported instruction: {}",
                llvm_print_value_to_string(term).unwrap_or_default()
            )
        }
    }
}

/// Return RPO ordering of blocks in an LLVM function.
fn rpo(function: LLVMValue) -> Vec<LLVMBasicBlock> {
    let visited = &mut FxHashSet::<LLVMBasicBlock>::default();
    let mut po = Vec::<LLVMBasicBlock>::new();
    let mut revpo = Vec::<LLVMBasicBlock>::new();

    fn walk(
        block: LLVMBasicBlock,
        visited: &mut FxHashSet<LLVMBasicBlock>,
        po: &mut Vec<LLVMBasicBlock>,
    ) {
        if !visited.insert(block) {
            // block already visited.
            return;
        }
        // Visit successors before visiting self.
        for succ in successors(block).into_iter() {
            walk(succ, visited, po);
        }
        // Visit self.
        po.push(block);
    }

    // Walk every block (not just entry) since we may have unreachable blocks.
    for block in basic_block_iter(function) {
        if visited.contains(&block) {
            continue;
        }
        walk(block, visited, &mut po);
        // We collect the RPO for this connected component right now
        // so that the entry block of the function comes first in the final ordering.
        revpo.extend(po.drain(..).rev());
    }

    revpo
}

#[derive(Error, Debug)]
pub enum ConversionErr {
    #[error("Unable to get operand with idx {0}")]
    OpdMissing(usize),
    #[error("Unable to get successor with idx {0}")]
    SuccMissing(usize),
    #[error("PHI node must have argument from predecessor block \"^{0}\"")]
    PhiArgMissing(String),
    #[error("Definition for value \"{0}\" not seen yet")]
    UndefinedValue(String),
    #[error("Block definition \"^{0}\" not seen yet")]
    UndefinedBlock(String),
    #[error("Integer constant has bit-width 0")]
    ZeroWidthIntConst,
    #[error("Floating point constant not of floating point type")]
    FloatConstNotFloatType,
    #[error("Switch case value is not an integer constant")]
    SwitchCaseNonIntConst,
}

/// If a value is a ConstantOp with integer type, return the value.
fn get_const_op_as_int(ctx: &Context, val: Value) -> Option<IntegerAttr> {
    let defining_op = val.defining_op()?;

    Operation::get_op::<ConstantOp>(defining_op, ctx).and_then(|const_op| {
        const_op
            .get_value(ctx)
            .downcast_ref::<IntegerAttr>()
            .cloned()
    })
}

/// If a value is a ConstantOp with a 32-bit integer type, return the value.
fn get_const_op_as_u32(ctx: &Context, val: Value) -> Option<u32> {
    get_const_op_as_int(ctx, val).and_then(|int_attr| {
        let int_ty = int_attr.get_type().deref(ctx);
        // LLVM integers are signless.
        (int_ty.is_signless() && int_ty.width() == 32).then(|| int_attr.value().to_u32())
    })
}

/// Checks if a constant has been processed already, and if not
/// converts it and puts it in the [ConversionContext::value_map].
fn process_constant(ctx: &mut Context, cctx: &mut ConversionContext, val: LLVMValue) -> Result<()> {
    if cctx.value_map.contains_key(&val) {
        return Ok(());
    }
    let ll_ty = llvm_type_of(val);
    let ty = convert_type(ctx, cctx, ll_ty)?;

    // Insert a new constant instruction in the entry block.
    fn insert_const_inst(ctx: &mut Context, cctx: &mut ConversionContext, op: Ptr<Operation>) {
        cctx.constants_inserter
            .as_mut()
            .unwrap()
            .append_operation(ctx, op);
    }

    match llvm_get_value_kind(val) {
        LLVMValueKind::LLVMUndefValueValueKind => {
            let undef_op = UndefOp::new(ctx, ty);
            insert_const_inst(ctx, cctx, undef_op.get_operation());
            cctx.value_map.insert(val, undef_op.get_result(ctx));
        }
        LLVMValueKind::LLVMPoisonValueKind => {
            let poison_op = PoisonOp::new(ctx, ty);
            insert_const_inst(ctx, cctx, poison_op.get_operation());
            cctx.value_map.insert(val, poison_op.get_result(ctx));
        }
        LLVMValueKind::LLVMConstantPointerNullValueKind => {
            let null_op = ZeroOp::new(ctx, ty);
            insert_const_inst(ctx, cctx, null_op.get_operation());
            cctx.value_map.insert(val, null_op.get_result(ctx));
        }
        LLVMValueKind::LLVMConstantIntValueKind => {
            // TODO: Zero extend or sign extend?
            let u64 = llvm_const_int_get_zext_value(val);
            let int_ty = TypedHandle::<IntegerType>::from_handle(ty, ctx)?;
            let width = int_ty.deref(ctx).width() as usize;
            if width == 0 {
                return input_err_noloc!(ConversionErr::ZeroWidthIntConst);
            }
            let val_attr =
                IntegerAttr::new(int_ty, APInt::from_u64(u64, NonZero::new(width).unwrap()));
            let const_op = ConstantOp::new(ctx, Box::new(val_attr));
            insert_const_inst(ctx, cctx, const_op.get_operation());
            cctx.value_map.insert(val, const_op.get_result(ctx));
        }
        LLVMValueKind::LLVMConstantFPValueKind => {
            let (fp64, lost_info) = llvm_const_real_get_double(val);
            assert!(!lost_info, "Lost information when converting FP constant");
            let val_attr: AttrObj = {
                let float_ty = &*ty.deref(ctx);
                let Some(float_ty_attr): Option<&dyn FloatAttrBuilder> = type_cast(float_ty) else {
                    return input_err_noloc!(ConversionErr::FloatConstNotFloatType);
                };
                float_ty_attr.value_from_f64(fp64)
            };
            let const_op = ConstantOp::new(ctx, val_attr);
            insert_const_inst(ctx, cctx, const_op.get_operation());
            cctx.value_map.insert(val, const_op.get_result(ctx));
        }
        LLVMValueKind::LLVMConstantArrayValueKind
        | LLVMValueKind::LLVMConstantDataArrayValueKind => {
            fn get_operand(val: LLVMValue, index: u32) -> Result<LLVMValue> {
                if matches!(
                    llvm_get_value_kind(val),
                    LLVMValueKind::LLVMConstantDataArrayValueKind
                ) {
                    llvm_get_aggregate_element(val, index).ok_or_else(|| {
                        input_error_noloc!("LLVMConstantDataArrayValueKind does not have an element at index {index}")
                    })
                } else {
                    Ok(llvm_get_operand(val, index))
                }
            }
            let mut field_vals = vec![];
            let num_elements = llvm_get_array_length2(ll_ty);
            for i in 0..num_elements {
                let field_val = get_operand(val, i.try_into().unwrap())?;
                process_constant(ctx, cctx, field_val)?;
                let Some(m_val) = cctx.value_map.get(&field_val) else {
                    panic!("We just processed this constant, it must be in the map");
                };
                field_vals.push(*m_val);
            }
            // Starting with an Undef value, we insert elements, for each field.
            let undef_op = UndefOp::new(ctx, ty);
            insert_const_inst(ctx, cctx, undef_op.get_operation());
            let (ctx, const_array) = field_vals.iter().enumerate().try_fold(
                (ctx, undef_op.get_operation()),
                |(ctx, acc), (field_idx, field_val)| -> Result<_> {
                    let acc_val = acc.deref(ctx).get_result(0);
                    let insert_op = InsertValueOp::new(
                        ctx,
                        acc_val,
                        *field_val,
                        vec![field_idx.try_into().unwrap()],
                    )
                    .get_operation();
                    insert_const_inst(ctx, cctx, insert_op);
                    Ok((ctx, insert_op))
                },
            )?;

            cctx.value_map
                .insert(val, const_array.deref(ctx).get_result(0));
        }
        LLVMValueKind::LLVMConstantStructValueKind => {
            let mut field_vals = vec![];
            let num_fields = llvm_count_struct_element_types(ll_ty);
            for i in 0..num_fields {
                let field_val = llvm_get_operand(val, i);
                process_constant(ctx, cctx, field_val)?;
                let Some(m_val) = cctx.value_map.get(&field_val) else {
                    panic!("We just processed this constant, it must be in the map");
                };
                field_vals.push(*m_val);
            }
            // Starting with an Undef value, we insert elements, for each field.
            let undef_op = UndefOp::new(ctx, ty);
            insert_const_inst(ctx, cctx, undef_op.get_operation());
            let (ctx, const_struct) = field_vals.iter().enumerate().try_fold(
                (ctx, undef_op.get_operation()),
                |(ctx, acc), (field_idx, field_val)| -> Result<_> {
                    let acc_val = acc.deref(ctx).get_result(0);
                    let insert_op = InsertValueOp::new(
                        ctx,
                        acc_val,
                        *field_val,
                        vec![field_idx.try_into().unwrap()],
                    )
                    .get_operation();
                    insert_const_inst(ctx, cctx, insert_op);
                    Ok((ctx, insert_op))
                },
            )?;

            cctx.value_map
                .insert(val, const_struct.deref(ctx).get_result(0));
        }
        LLVMValueKind::LLVMConstantExprValueKind => {
            let opcode = llvm_get_const_opcode(val);
            match opcode {
                LLVMOpcode::LLVMBitCast => {
                    let opd = llvm_get_operand(val, 0);
                    process_constant(ctx, cctx, opd)?;
                    let Some(m_val) = cctx.value_map.get(&opd) else {
                        panic!("We just processed this constant, it must be in the map");
                    };
                    let cast_op = BitcastOp::new(ctx, *m_val, ty);
                    insert_const_inst(ctx, cctx, cast_op.get_operation());
                    cctx.value_map.insert(val, cast_op.get_result(ctx));
                }
                LLVMOpcode::LLVMIntToPtr => {
                    let opd = llvm_get_operand(val, 0);
                    process_constant(ctx, cctx, opd)?;
                    let Some(m_val) = cctx.value_map.get(&opd) else {
                        panic!("We just processed this constant, it must be in the map");
                    };
                    let cast_op = IntToPtrOp::new(ctx, *m_val, ty);
                    insert_const_inst(ctx, cctx, cast_op.get_operation());
                    cctx.value_map.insert(val, cast_op.get_result(ctx));
                }
                LLVMOpcode::LLVMPtrToInt => {
                    let opd = llvm_get_operand(val, 0);
                    process_constant(ctx, cctx, opd)?;
                    let Some(m_val) = cctx.value_map.get(&opd) else {
                        panic!("We just processed this constant, it must be in the map");
                    };
                    let cast_op = PtrToIntOp::new(ctx, *m_val, ty);
                    insert_const_inst(ctx, cctx, cast_op.get_operation());
                    cctx.value_map.insert(val, cast_op.get_result(ctx));
                }
                LLVMOpcode::LLVMTrunc => {
                    let opd = llvm_get_operand(val, 0);
                    process_constant(ctx, cctx, opd)?;
                    let Some(m_val) = cctx.value_map.get(&opd).cloned() else {
                        panic!("We just processed this constant, it must be in the map");
                    };
                    let trunc_op = TruncOp::new(ctx, m_val, ty);
                    insert_const_inst(ctx, cctx, trunc_op.get_operation());
                    cctx.value_map.insert(val, trunc_op.get_result(ctx));
                }
                LLVMOpcode::LLVMGetElementPtr => {
                    let base = llvm_get_operand(val, 0);
                    process_constant(ctx, cctx, base)?;
                    let Some(m_base) = cctx.value_map.get(&base).cloned() else {
                        panic!("We just processed this constant, it must be in the map");
                    };
                    let mut indices = vec![];
                    for i in 1..llvm_get_num_operands(val) {
                        let opd = llvm_get_operand(val, i);
                        process_constant(ctx, cctx, opd)?;
                        let Some(m_val) = cctx.value_map.get(&opd) else {
                            panic!("We just processed this constant, it must be in the map");
                        };
                        if let Some(c) = get_const_op_as_u32(ctx, *m_val) {
                            indices.push(GepIndex::Constant(c));
                        } else {
                            indices.push(GepIndex::Value(*m_val));
                        }
                    }
                    let src_elm_type =
                        convert_type(ctx, cctx, llvm_get_gep_source_element_type(val))?;
                    let gep_op = GetElementPtrOp::new(ctx, m_base, indices, src_elm_type);
                    insert_const_inst(ctx, cctx, gep_op.get_operation());
                    cctx.value_map.insert(val, gep_op.get_result(ctx));
                }
                LLVMOpcode::LLVMAdd | LLVMOpcode::LLVMSub => {
                    let (lhs, rhs) = (llvm_get_operand(val, 0), llvm_get_operand(val, 1));
                    process_constant(ctx, cctx, lhs)?;
                    process_constant(ctx, cctx, rhs)?;
                    let Some(m_lhs) = cctx.value_map.get(&lhs).cloned() else {
                        panic!("We just processed this constant, it must be in the map");
                    };
                    let Some(m_rhs) = cctx.value_map.get(&rhs).cloned() else {
                        panic!("We just processed this constant, it must be in the map");
                    };
                    // `Instruction *ConstantExpr::getAsInstruction() const ` just sets no flags.
                    let flags = IntegerOverflowFlagsAttr::default();
                    let (op, res_val) = if opcode == LLVMOpcode::LLVMAdd {
                        let op = AddOp::new(ctx, m_lhs, m_rhs);
                        op.set_integer_overflow_flag(ctx, flags);
                        (op.get_operation(), op.get_result(ctx))
                    } else {
                        let op = SubOp::new(ctx, m_lhs, m_rhs);
                        op.set_integer_overflow_flag(ctx, flags);
                        (op.get_operation(), op.get_result(ctx))
                    };
                    insert_const_inst(ctx, cctx, op);
                    cctx.value_map.insert(val, res_val);
                }
                _ => {
                    todo!("Unsupported constant expression opcode: {:?}", opcode)
                }
            }
        }
        LLVMValueKind::LLVMGlobalVariableValueKind => {
            let global_name = llvm_get_value_name(val).unwrap_or_default();
            let global_name = cctx.id_legaliser.legalise(&global_name);
            let global_op = AddressOfOp::new(
                ctx,
                global_name,
                llvm_get_pointer_address_space(llvm_type_of(val)),
            );
            insert_const_inst(ctx, cctx, global_op.get_operation());
            cctx.value_map.insert(val, global_op.get_result(ctx));
        }
        LLVMValueKind::LLVMFunctionValueKind => {
            let fn_name = llvm_get_value_name(val).unwrap_or_default();
            let fn_name = cctx.id_legaliser.legalise(&fn_name);
            let func_op = AddressOfOp::new(
                ctx,
                fn_name,
                llvm_get_pointer_address_space(llvm_type_of(val)),
            );
            insert_const_inst(ctx, cctx, func_op.get_operation());
            cctx.value_map.insert(val, func_op.get_result(ctx));
        }
        LLVMValueKind::LLVMConstantAggregateZeroValueKind => {
            let zero_op = ZeroOp::new(ctx, ty);
            insert_const_inst(ctx, cctx, zero_op.get_operation());
            cctx.value_map.insert(val, zero_op.get_result(ctx));
        }
        LLVMValueKind::LLVMConstantVectorValueKind
        | LLVMValueKind::LLVMConstantDataVectorValueKind => {
            let num_elements = llvm_get_vector_size(ll_ty);
            let mut element_vals = vec![];
            for i in 0..num_elements {
                let element_val = llvm_get_aggregate_element(val, i)
                    .expect("Constant vector must have element at index");
                process_constant(ctx, cctx, element_val)?;
                let Some(m_val) = cctx.value_map.get(&element_val) else {
                    panic!("We just processed this constant, it must be in the map");
                };
                element_vals.push(*m_val);
            }
            // Starting with an Undef value, we insert elements, for each element.
            let undef_op = UndefOp::new(ctx, ty);
            let ty_int32 = TypedHandle::<IntegerType>::from_handle(
                IntegerType::get(ctx, 32, Signedness::Signless).into(),
                ctx,
            )?;
            insert_const_inst(ctx, cctx, undef_op.get_operation());
            let const_vector = element_vals.iter().enumerate().try_fold(
                undef_op.get_operation(),
                |acc, (elem_idx, elem_val)| -> Result<_> {
                    let acc_val = acc.deref(ctx).get_result(0);
                    // The index needs to be a [Value], so we create a ConstantOp for it.
                    let idx_attr = IntegerAttr::new(
                        ty_int32,
                        APInt::from_u64(elem_idx.try_into().unwrap(), NonZero::new(32).unwrap()),
                    );
                    let idx_const_op = ConstantOp::new(ctx, Box::new(idx_attr)).get_operation();
                    insert_const_inst(ctx, cctx, idx_const_op);
                    let idx_val = idx_const_op.deref(ctx).get_result(0);
                    let insert_op =
                        InsertElementOp::new(ctx, acc_val, *elem_val, idx_val).get_operation();
                    insert_const_inst(ctx, cctx, insert_op);
                    Ok(insert_op)
                },
            )?;
            cctx.value_map
                .insert(val, const_vector.deref(ctx).get_result(0));
        }
        LLVMValueKind::LLVMArgumentValueKind => todo!(),
        LLVMValueKind::LLVMBasicBlockValueKind => todo!(),
        LLVMValueKind::LLVMMemoryUseValueKind => todo!(),
        LLVMValueKind::LLVMMemoryDefValueKind => todo!(),
        LLVMValueKind::LLVMMemoryPhiValueKind => todo!(),
        LLVMValueKind::LLVMGlobalAliasValueKind => todo!(),
        LLVMValueKind::LLVMGlobalIFuncValueKind => todo!(),
        LLVMValueKind::LLVMBlockAddressValueKind => todo!(),
        LLVMValueKind::LLVMConstantTokenNoneValueKind => todo!(),
        LLVMValueKind::LLVMMetadataAsValueValueKind => todo!(),
        LLVMValueKind::LLVMInlineAsmValueKind => todo!(),
        LLVMValueKind::LLVMInstructionValueKind => todo!(),
        LLVMValueKind::LLVMConstantTargetNoneValueKind => todo!(),
        LLVMValueKind::LLVMConstantPtrAuthValueKind => todo!(),
    }
    Ok(())
}

fn convert_operands(
    ctx: &mut Context,
    cctx: &mut ConversionContext,
    operands: &[LLVMValue],
) -> Result<(Vec<Value>, Vec<Ptr<BasicBlock>>)> {
    let mut opds = vec![];
    let mut succs = vec![];

    for opd in operands.iter().cloned() {
        if !llvm_value_is_basic_block(opd) {
            process_constant(ctx, cctx, opd)?;
            if let Some(m_val) = cctx.value_map.get(&opd) {
                opds.push(*m_val);
            } else {
                return input_err_noloc!(ConversionErr::UndefinedValue(
                    llvm_get_value_name(opd).unwrap_or_default()
                ));
            }
        } else {
            let block = llvm_value_as_basic_block(opd);
            let Some(m_block) = cctx.block_map.get(&block) else {
                return input_err_noloc!(ConversionErr::UndefinedBlock(
                    llvm_get_basic_block_name(block).unwrap_or_default()
                ));
            };
            succs.push(*m_block);
        }
    }
    Ok((opds, succs))
}

fn get_operand<T: Clone>(opds: &[T], idx: usize) -> Result<T> {
    opds.get(idx)
        .ok_or_else(|| input_error_noloc!(ConversionErr::OpdMissing(idx)))
        .cloned()
}

/// Compute the arguments to be passed when branching from `src` to `dest`.
fn convert_branch_args(
    ctx: &mut Context,
    cctx: &mut ConversionContext,
    src_block: LLVMBasicBlock,
    dst_block: LLVMBasicBlock,
) -> Result<Vec<Value>> {
    let mut args = vec![];
    for inst in instruction_iter(dst_block) {
        if llvm_is_a::phi_node(inst) {
            let Some((incoming_val, _)) =
                incoming_iter(inst).find(|(_, block)| *block == src_block)
            else {
                return input_err_noloc!(ConversionErr::PhiArgMissing(
                    llvm_get_basic_block_name(src_block).unwrap_or_default()
                ));
            };
            process_constant(ctx, cctx, incoming_val)?;
            let Some(m_incoming_val) = cctx.value_map.get(&incoming_val) else {
                return input_err_noloc!(ConversionErr::UndefinedValue(
                    llvm_get_value_name(incoming_val).unwrap_or_default()
                ));
            };
            args.push(*m_incoming_val)
        } else {
            // PHIs are at the start of the block.
            break;
        }
    }
    Ok(args)
}

/// Map an LLVM-C [LLVMAtomicOrdering] to a pliron [AtomicOrderingAttr].
fn convert_ordering_from_llvm(o: LLVMAtomicOrdering) -> AtomicOrderingAttr {
    match o {
        LLVMAtomicOrdering::LLVMAtomicOrderingMonotonic => AtomicOrderingAttr::Monotonic,
        LLVMAtomicOrdering::LLVMAtomicOrderingAcquire => AtomicOrderingAttr::Acquire,
        LLVMAtomicOrdering::LLVMAtomicOrderingRelease => AtomicOrderingAttr::Release,
        LLVMAtomicOrdering::LLVMAtomicOrderingAcquireRelease => AtomicOrderingAttr::AcqRel,
        LLVMAtomicOrdering::LLVMAtomicOrderingSequentiallyConsistent => AtomicOrderingAttr::SeqCst,
        other => panic!("unsupported atomic ordering from LLVM: {other:?}"),
    }
}

/// Map an LLVM-C [LLVMAtomicRMWBinOp] to a pliron [AtomicRmwKindAttr].
fn convert_rmw_kind_from_llvm(k: LLVMAtomicRMWBinOp) -> AtomicRmwKindAttr {
    match k {
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpXchg => AtomicRmwKindAttr::Xchg,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpAdd => AtomicRmwKindAttr::Add,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpSub => AtomicRmwKindAttr::Sub,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpAnd => AtomicRmwKindAttr::And,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpNand => AtomicRmwKindAttr::Nand,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpOr => AtomicRmwKindAttr::Or,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpXor => AtomicRmwKindAttr::Xor,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpMax => AtomicRmwKindAttr::Max,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpMin => AtomicRmwKindAttr::Min,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpUMax => AtomicRmwKindAttr::UMax,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpUMin => AtomicRmwKindAttr::UMin,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpFAdd => AtomicRmwKindAttr::FAdd,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpFSub => AtomicRmwKindAttr::FSub,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpFMax => AtomicRmwKindAttr::FMax,
        LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpFMin => AtomicRmwKindAttr::FMin,
        other => panic!("unsupported atomicrmw binop from LLVM: {other:?}"),
    }
}

/// Recover pliron syncscope (`None` = system) from an atomic instruction.
///
/// LLVM's built-in `SyncScope` ids are `SingleThread = 0` and `System = 1`;
/// any named scope (e.g. NVVM "device"/"block") is assigned an id >= 2. LLVM-C
/// exposes the id but not its name, so only the two built-in scopes round-trip;
/// named scopes cannot be mapped back to a name and fall back to the system
/// scope.
fn syncscope_from_llvm(inst: LLVMValue) -> Option<String> {
    match llvm_get_atomic_sync_scope_id(inst) {
        0 => Some("singlethread".to_string()),
        _ => None,
    }
}

fn convert_call(
    ctx: &mut Context,
    cctx: &mut ConversionContext,
    inst: LLVMValue,
) -> Result<Ptr<Operation>> {
    let llvm_operands: Vec<_> = (0..llvm_get_num_arg_operands(inst))
        .map(|opd_idx| llvm_get_operand(inst, opd_idx))
        .collect();
    let (args, _) = convert_operands(ctx, cctx, &llvm_operands)?;

    let callee = llvm_get_called_value(inst);

    // Inline asm: the callee is an inline-asm value rather than a function.
    if llvm_get_value_kind(callee) == LLVMValueKind::LLVMInlineAsmValueKind {
        let asm = llvm_get_inline_asm_asm_string(callee);
        let constraints = llvm_get_inline_asm_constraint_string(callee);
        let result_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
        // `convergent` is a call-site attribute, not recoverable through LLVM-C here.
        return Ok(
            InlineAsmOp::new(ctx, result_ty, args, &asm, &constraints, false).get_operation(),
        );
    }

    enum Callee {
        FnCall(CallOpCallable),
        IntrinsicCall(String),
    }
    let callee = if llvm_is_a::function(callee) {
        let llvm_fn_name =
            llvm_get_value_name(callee).expect("Unable to obtain valid function name");
        if llvm_lookup_intrinsic_id(&llvm_fn_name).is_some() {
            Callee::IntrinsicCall(llvm_fn_name)
        } else {
            let fn_name = cctx.id_legaliser.legalise(&llvm_fn_name);
            Callee::FnCall(CallOpCallable::Direct(fn_name))
        }
    } else {
        let (callee_converted, _) = convert_operands(ctx, cctx, &[callee])?;
        Callee::FnCall(CallOpCallable::Indirect(callee_converted[0]))
    };

    let callee_ty = llvm_get_called_function_type(inst);
    let callee_ty: TypedHandle<FuncType> =
        convert_type(ctx, cctx, callee_ty).and_then(|ty| TypedHandle::from_handle(ty, ctx))?;

    let fmf: Option<FastmathFlagsAttr> = if llvm_can_value_use_fast_math_flags(inst) {
        // Not all calls can have fast-math flags.
        let fmf = llvm_get_fast_math_flags(inst);
        (!fmf.is_empty()).then_some(fmf.into())
    } else {
        None
    };

    let op = match callee {
        Callee::FnCall(callable) => {
            let op = CallOp::new(ctx, callable, callee_ty, args);
            if let Some(fmf) = fmf {
                op.set_attr_llvm_call_fastmath_flags(ctx, fmf);
            }
            op.get_operation()
        }
        Callee::IntrinsicCall(name) => {
            let op = CallIntrinsicOp::new(ctx, name.into(), callee_ty, args);
            if let Some(fmf) = fmf {
                op.set_attr_llvm_intrinsic_fastmath_flags(ctx, fmf);
            }
            op.get_operation()
        }
    };

    Ok(op)
}

fn convert_instruction(
    ctx: &mut Context,
    cctx: &mut ConversionContext,
    inst: LLVMValue,
) -> Result<Ptr<Operation>> {
    if llvm_is_a::call_inst(inst) {
        return convert_call(ctx, cctx, inst);
    }

    fn get_integer_overflow_flag(inst: LLVMValue) -> IntegerOverflowFlagsAttr {
        let mut flags = IntegerOverflowFlagsAttr::default();
        if llvm_get_nsw(inst) {
            flags.nsw = true;
        }
        if llvm_get_nuw(inst) {
            flags.nuw = true;
        }
        flags
    }

    let llvm_operands: Vec<_> = (0..llvm_get_num_operands(inst))
        .map(|opd_idx| llvm_get_operand(inst, opd_idx))
        .collect();

    let (ref opds, ref succs) = convert_operands(ctx, cctx, &llvm_operands)?;
    match llvm_get_instruction_opcode(inst) {
        LLVMOpcode::LLVMAdd => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(
                AddOp::new_with_overflow_flag(ctx, lhs, rhs, get_integer_overflow_flag(inst))
                    .get_operation(),
            )
        }
        LLVMOpcode::LLVMAddrSpaceCast => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            Ok(AddrSpaceCastOp::new(ctx, arg, res_ty).get_operation())
        }
        LLVMOpcode::LLVMAlloca => {
            let elem_type = convert_type(ctx, cctx, llvm_get_allocated_type(inst))?;
            let size = get_operand(opds, 0)?;
            let op = AllocaOp::new(ctx, elem_type, size);
            let alignment = llvm_get_alignment(inst);
            if alignment != 0 {
                op.set_alignment(ctx, alignment);
            }
            Ok(op.get_operation())
        }
        LLVMOpcode::LLVMAnd => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(AndOp::new(ctx, lhs, rhs).get_operation())
        }
        LLVMOpcode::LLVMAShr => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(AShrOp::new(ctx, lhs, rhs).get_operation())
        }
        LLVMOpcode::LLVMAtomicCmpXchg => {
            let (ptr, cmp, new) = (
                get_operand(opds, 0)?,
                get_operand(opds, 1)?,
                get_operand(opds, 2)?,
            );
            let success = convert_ordering_from_llvm(llvm_get_cmpxchg_success_ordering(inst));
            let failure = convert_ordering_from_llvm(llvm_get_cmpxchg_failure_ordering(inst));
            let syncscope = syncscope_from_llvm(inst);
            Ok(
                AtomicCmpxchgOp::new(ctx, ptr, cmp, new, success, failure, syncscope)
                    .get_operation(),
            )
        }
        LLVMOpcode::LLVMAtomicRMW => {
            let (ptr, val) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            let kind = convert_rmw_kind_from_llvm(llvm_get_atomic_rmw_bin_op(inst));
            let ordering = convert_ordering_from_llvm(llvm_get_ordering(inst));
            let syncscope = syncscope_from_llvm(inst);
            Ok(AtomicRmwOp::new(ctx, ptr, val, kind, ordering, syncscope).get_operation())
        }
        LLVMOpcode::LLVMBitCast => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            Ok(BitcastOp::new(ctx, arg, res_ty).get_operation())
        }
        LLVMOpcode::LLVMBr => {
            if !opds.is_empty() {
                assert!(
                    succs.len() == 2,
                    "Conditional branch must have two successors"
                );
                let true_dest_opds = convert_branch_args(
                    ctx,
                    cctx,
                    llvm_get_instruction_parent(inst).unwrap(),
                    llvm_value_as_basic_block(llvm_get_operand(inst, 2)),
                )?;
                let false_dest_opds = convert_branch_args(
                    ctx,
                    cctx,
                    llvm_get_instruction_parent(inst).unwrap(),
                    llvm_value_as_basic_block(llvm_get_operand(inst, 1)),
                )?;
                Ok(CondBrOp::new(
                    ctx,
                    get_operand(opds, 0)?,
                    get_operand(succs, 1)?,
                    true_dest_opds,
                    get_operand(succs, 0)?,
                    false_dest_opds,
                )
                .get_operation())
            } else {
                let dest_opds = convert_branch_args(
                    ctx,
                    cctx,
                    llvm_get_instruction_parent(inst).unwrap(),
                    llvm_value_as_basic_block(llvm_get_operand(inst, 0)),
                )?;
                Ok(BrOp::new(ctx, get_operand(succs, 0)?, dest_opds).get_operation())
            }
        }
        LLVMOpcode::LLVMCall => {
            unreachable!("Should've already been processed separately")
        }
        LLVMOpcode::LLVMCallBr => todo!(),
        LLVMOpcode::LLVMCatchPad => todo!(),
        LLVMOpcode::LLVMCatchRet => todo!(),
        LLVMOpcode::LLVMCatchSwitch => todo!(),
        LLVMOpcode::LLVMCleanupPad => todo!(),
        LLVMOpcode::LLVMCleanupRet => todo!(),
        LLVMOpcode::LLVMFNeg => {
            let arg = get_operand(opds, 0)?;
            Ok(
                FNegOp::new_with_fast_math_flags(ctx, arg, llvm_get_fast_math_flags(inst).into())
                    .get_operation(),
            )
        }
        LLVMOpcode::LLVMFAdd => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(FAddOp::new_with_fast_math_flags(
                ctx,
                lhs,
                rhs,
                llvm_get_fast_math_flags(inst).into(),
            )
            .get_operation())
        }
        LLVMOpcode::LLVMFCmp => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            let pred = convert_fpredicate(llvm_get_fcmp_predicate(inst));
            let fastmath = llvm_get_fast_math_flags(inst);
            let op = FCmpOp::new(ctx, pred, lhs, rhs);
            op.set_fast_math_flags(ctx, fastmath.into());
            Ok(op.get_operation())
        }
        LLVMOpcode::LLVMFDiv => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(FDivOp::new_with_fast_math_flags(
                ctx,
                lhs,
                rhs,
                llvm_get_fast_math_flags(inst).into(),
            )
            .get_operation())
        }
        LLVMOpcode::LLVMFMul => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(FMulOp::new_with_fast_math_flags(
                ctx,
                lhs,
                rhs,
                llvm_get_fast_math_flags(inst).into(),
            )
            .get_operation())
        }
        LLVMOpcode::LLVMFRem => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(FRemOp::new_with_fast_math_flags(
                ctx,
                lhs,
                rhs,
                llvm_get_fast_math_flags(inst).into(),
            )
            .get_operation())
        }
        LLVMOpcode::LLVMFSub => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(FSubOp::new_with_fast_math_flags(
                ctx,
                lhs,
                rhs,
                llvm_get_fast_math_flags(inst).into(),
            )
            .get_operation())
        }
        LLVMOpcode::LLVMFPExt => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            let op = FPExtOp::new(ctx, arg, res_ty);
            op.set_fast_math_flags(ctx, llvm_get_fast_math_flags(inst).into());
            Ok(op.get_operation())
        }
        LLVMOpcode::LLVMFPTrunc => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            let op = FPTruncOp::new(ctx, arg, res_ty);
            op.set_fast_math_flags(ctx, llvm_get_fast_math_flags(inst).into());
            Ok(op.get_operation())
        }
        LLVMOpcode::LLVMFPToSI => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            Ok(FPToSIOp::new(ctx, arg, res_ty).get_operation())
        }
        LLVMOpcode::LLVMFPToUI => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            Ok(FPToUIOp::new(ctx, arg, res_ty).get_operation())
        }
        LLVMOpcode::LLVMSIToFP => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            Ok(SIToFPOp::new(ctx, arg, res_ty).get_operation())
        }
        LLVMOpcode::LLVMUIToFP => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            let nneg = llvm_get_nneg(inst);
            Ok(UIToFPOp::new_with_nneg(ctx, arg, res_ty, nneg).get_operation())
        }
        LLVMOpcode::LLVMFence => {
            let ordering = convert_ordering_from_llvm(llvm_get_ordering(inst));
            let syncscope = syncscope_from_llvm(inst);
            Ok(FenceOp::new(ctx, ordering, syncscope).get_operation())
        }
        LLVMOpcode::LLVMFreeze => {
            let arg = get_operand(opds, 0)?;
            Ok(FreezeOp::new(ctx, arg).get_operation())
        }
        LLVMOpcode::LLVMGetElementPtr => {
            let mut opds = opds.iter();
            let base = opds
                .next()
                .ok_or_else(|| input_error_noloc!(ConversionErr::OpdMissing(0)))?;
            let indices = opds
                .map(|v| {
                    if let Some(c) = get_const_op_as_u32(ctx, *v) {
                        GepIndex::Constant(c)
                    } else {
                        GepIndex::Value(*v)
                    }
                })
                .collect::<Vec<_>>();
            let src_elm_type = convert_type(ctx, cctx, llvm_get_gep_source_element_type(inst))?;
            Ok(GetElementPtrOp::new(ctx, *base, indices, src_elm_type).get_operation())
        }
        LLVMOpcode::LLVMICmp => {
            let pred = convert_ipredicate(llvm_get_icmp_predicate(inst));
            Ok(
                ICmpOp::new(ctx, pred, get_operand(opds, 0)?, get_operand(opds, 1)?)
                    .get_operation(),
            )
        }
        LLVMOpcode::LLVMIndirectBr => todo!(),
        LLVMOpcode::LLVMInsertElement => {
            let vector = get_operand(opds, 0)?;
            let element = get_operand(opds, 1)?;
            let index = get_operand(opds, 2)?;
            Ok(InsertElementOp::new(ctx, vector, element, index).get_operation())
        }
        LLVMOpcode::LLVMExtractElement => {
            let vector = get_operand(opds, 0)?;
            let index = get_operand(opds, 1)?;
            Ok(ExtractElementOp::new(ctx, vector, index).get_operation())
        }
        LLVMOpcode::LLVMShuffleVector => {
            let vec1 = get_operand(opds, 0)?;
            let vec2 = get_operand(opds, 1)?;
            let num_mask_elems = llvm_get_num_mask_elements(inst);
            let mask = (0..num_mask_elems)
                .map(|i| llvm_get_mask_value(inst, i))
                .collect();
            Ok(ShuffleVectorOp::new(ctx, vec1, vec2, mask).get_operation())
        }
        LLVMOpcode::LLVMInsertValue => {
            let (aggr, val) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            let indices = llvm_get_indices(inst);
            Ok(InsertValueOp::new(ctx, aggr, val, indices).get_operation())
        }
        LLVMOpcode::LLVMExtractValue => {
            let aggr = get_operand(opds, 0)?;
            let indices = llvm_get_indices(inst);
            Ok(ExtractValueOp::new(ctx, aggr, indices)?.get_operation())
        }
        LLVMOpcode::LLVMIntToPtr => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            Ok(IntToPtrOp::new(ctx, arg, res_ty).get_operation())
        }
        LLVMOpcode::LLVMInvoke => todo!(),
        LLVMOpcode::LLVMLandingPad => todo!(),
        LLVMOpcode::LLVMLoad => {
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            let ptr = get_operand(opds, 0)?;
            let alignment = llvm_get_alignment(inst);
            let ordering = llvm_get_ordering(inst);
            if matches!(ordering, LLVMAtomicOrdering::LLVMAtomicOrderingNotAtomic) {
                let load_op = LoadOp::new(ctx, ptr, res_ty);
                if alignment != 0 {
                    load_op.set_alignment(ctx, alignment);
                }
                Ok(load_op.get_operation())
            } else {
                let load_op = AtomicLoadOp::new(
                    ctx,
                    ptr,
                    res_ty,
                    convert_ordering_from_llvm(ordering),
                    syncscope_from_llvm(inst),
                );
                if alignment != 0 {
                    load_op.set_alignment(ctx, alignment);
                }
                Ok(load_op.get_operation())
            }
        }
        LLVMOpcode::LLVMLShr => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(LShrOp::new(ctx, lhs, rhs).get_operation())
        }
        LLVMOpcode::LLVMMul => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(
                MulOp::new_with_overflow_flag(ctx, lhs, rhs, get_integer_overflow_flag(inst))
                    .get_operation(),
            )
        }
        LLVMOpcode::LLVMOr => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(OrOp::new(ctx, lhs, rhs).get_operation())
        }
        LLVMOpcode::LLVMPHI => {
            unreachable!("PHI nodes must already be handled")
        }
        LLVMOpcode::LLVMPtrToInt => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            Ok(PtrToIntOp::new(ctx, arg, res_ty).get_operation())
        }
        LLVMOpcode::LLVMPtrToAddr => {
            unimplemented!("LLVM-C does not have LLVMBuildPtrToAddr yet")
        }
        LLVMOpcode::LLVMResume => todo!(),
        LLVMOpcode::LLVMRet => {
            let retval = if llvm_get_num_operands(inst) == 1 {
                Some(get_operand(opds, 0)?)
            } else {
                None
            };
            Ok(ReturnOp::new(ctx, retval).get_operation())
        }
        LLVMOpcode::LLVMSDiv => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(SDivOp::new(ctx, lhs, rhs).get_operation())
        }
        LLVMOpcode::LLVMSelect => {
            let (cond, true_val, false_val) = (
                get_operand(opds, 0)?,
                get_operand(opds, 1)?,
                get_operand(opds, 2)?,
            );
            Ok(SelectOp::new(ctx, cond, true_val, false_val).get_operation())
        }
        LLVMOpcode::LLVMSExt => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            Ok(SExtOp::new(ctx, arg, res_ty).get_operation())
        }
        LLVMOpcode::LLVMZExt => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            let nneg = llvm_get_nneg(inst);
            Ok(ZExtOp::new_with_nneg(ctx, arg, res_ty, nneg).get_operation())
        }
        LLVMOpcode::LLVMTrunc => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            Ok(TruncOp::new(ctx, arg, res_ty).get_operation())
        }
        LLVMOpcode::LLVMShl => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(
                ShlOp::new_with_overflow_flag(ctx, lhs, rhs, get_integer_overflow_flag(inst))
                    .get_operation(),
            )
        }
        LLVMOpcode::LLVMSRem => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(SRemOp::new(ctx, lhs, rhs).get_operation())
        }
        LLVMOpcode::LLVMStore => {
            let (value_opd, ptr_opd) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            let alignment = llvm_get_alignment(inst);
            let ordering = llvm_get_ordering(inst);
            if matches!(ordering, LLVMAtomicOrdering::LLVMAtomicOrderingNotAtomic) {
                let store_op = StoreOp::new(ctx, value_opd, ptr_opd);
                if alignment != 0 {
                    store_op.set_alignment(ctx, alignment);
                }
                Ok(store_op.get_operation())
            } else {
                let store_op = AtomicStoreOp::new(
                    ctx,
                    value_opd,
                    ptr_opd,
                    convert_ordering_from_llvm(ordering),
                    syncscope_from_llvm(inst),
                );
                if alignment != 0 {
                    store_op.set_alignment(ctx, alignment);
                }
                Ok(store_op.get_operation())
            }
        }
        LLVMOpcode::LLVMSub => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(
                SubOp::new_with_overflow_flag(ctx, lhs, rhs, get_integer_overflow_flag(inst))
                    .get_operation(),
            )
        }
        LLVMOpcode::LLVMSwitch => {
            let cond = get_operand(opds, 0)?;
            let default_dest = succs
                .first()
                .ok_or_else(|| input_error_noloc!(ConversionErr::SuccMissing(0)))?;
            let num_cases = succs.len() - 1;
            // Case values aren't stored as operands, but we can get them using `llvm_get_switch_case_value`.
            let mut case_values = vec![];
            // Case 0 is the default destination, so we start with 1.
            for case_idx in 1..=num_cases {
                let case_value = llvm_get_switch_case_value(inst, case_idx.try_into().unwrap());
                process_constant(ctx, cctx, case_value)?;
                let Some(case_value) = cctx.value_map.get(&case_value) else {
                    return input_err_noloc!(ConversionErr::SwitchCaseNonIntConst);
                };
                case_values.push(*case_value);
            }
            let cases = case_values
                .iter()
                // Skip the first successor which is the default destination
                .zip(succs.iter().skip(1))
                .enumerate()
                .map(|(case_idx, (case_val, dest_block))| {
                    let case_val = get_const_op_as_int(ctx, *case_val).ok_or_else(|| {
                        input_error_noloc!("Switch case value must be a constant integer")
                    })?;
                    let case_idx: u32 = case_idx.try_into().unwrap();
                    // Operand 0 is the condition and operand 1 is the default destination,
                    // so case destinations start at operand index 2.
                    let llvm_dest = llvm_value_as_basic_block(llvm_get_operand(inst, 2 + case_idx));
                    assert!(
                        cctx.block_map.get(&llvm_dest).unwrap() == dest_block,
                        "Switch case destination block does not match the expected block"
                    );
                    let case_args = convert_branch_args(
                        ctx,
                        cctx,
                        llvm_get_instruction_parent(inst).unwrap(),
                        llvm_dest,
                    )?;
                    Ok(SwitchCase {
                        value: case_val,
                        dest: *dest_block,
                        dest_opds: case_args,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let default_dest_args = convert_branch_args(
                ctx,
                cctx,
                llvm_get_instruction_parent(inst).unwrap(),
                llvm_value_as_basic_block(llvm_get_operand(inst, 1)),
            )?;
            Ok(SwitchOp::new(ctx, cond, *default_dest, default_dest_args, cases).get_operation())
        }
        LLVMOpcode::LLVMUDiv => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(UDivOp::new(ctx, lhs, rhs).get_operation())
        }
        LLVMOpcode::LLVMUnreachable => Ok(UnreachableOp::new(ctx).get_operation()),
        LLVMOpcode::LLVMURem => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(URemOp::new(ctx, lhs, rhs).get_operation())
        }
        LLVMOpcode::LLVMVAArg => {
            let arg = get_operand(opds, 0)?;
            let res_ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            Ok(VAArgOp::new(ctx, arg, res_ty).get_operation())
        }
        LLVMOpcode::LLVMUserOp1 => todo!(),
        LLVMOpcode::LLVMUserOp2 => todo!(),

        LLVMOpcode::LLVMXor => {
            let (lhs, rhs) = (get_operand(opds, 0)?, get_operand(opds, 1)?);
            Ok(XorOp::new(ctx, lhs, rhs).get_operation())
        }
    }
}

/// Convert [LLVMBasicBlock] to pliron's [BasicBlock].
fn convert_block(
    ctx: &mut Context,
    cctx: &mut ConversionContext,
    block: LLVMBasicBlock,
    m_block: Ptr<BasicBlock>,
) -> Result<()> {
    let mut inserter = IRInserter::<DummyListener>::new_at_block_end(m_block);
    for inst in instruction_iter(block) {
        if llvm_get_instruction_opcode(inst) == LLVMOpcode::LLVMPHI {
            let ty = convert_type(ctx, cctx, llvm_type_of(inst))?;
            let arg_idx = BasicBlock::push_argument(m_block, ctx, ty);
            cctx.value_map
                .insert(inst, m_block.deref(ctx).get_argument(arg_idx));
        } else {
            let m_inst = convert_instruction(ctx, cctx, inst)?;
            inserter.insert_operation(ctx, m_inst);
            let m_inst_result = m_inst.deref(ctx).results().next();
            // LLVM instructions have at most one result.
            if let Some(result) = m_inst_result {
                if let Some(res_name) = llvm_get_value_name(inst).filter(|name| !name.is_empty()) {
                    let res_name = cctx.id_legaliser.legalise(&res_name);
                    debug_info::set_operation_result_name(ctx, m_inst, 0, Some(res_name));
                }
                cctx.value_map.insert(inst, result);
            }
        }
    }
    Ok(())
}

fn convert_function(
    ctx: &mut Context,
    cctx: &mut ConversionContext,
    function: LLVMValue,
) -> Result<FuncOp> {
    assert!(llvm_is_a::function(function));

    let llvm_name = llvm_get_value_name(function).expect("Expected function to have a name");
    let name = cctx.id_legaliser.legalise(&llvm_name);
    let fn_ty = convert_type(ctx, cctx, llvm_global_get_value_type(function))?;
    let fn_ty = TypedHandle::from_handle(fn_ty, ctx)?;
    // Create a new FuncOp.
    let m_func = FuncOp::new(ctx, name.clone(), fn_ty);

    let linkage = convert_linkage(llvm_get_linkage(function));
    m_func.set_attr_llvm_function_linkage(ctx, linkage);

    if llvm_name != <Identifier as Into<String>>::into(name) {
        m_func.set_llvm_symbol_name(ctx, llvm_name);
    }

    // If function is just a declaration, we have nothing more to do.
    if llvm_is_declaration(function) {
        return Ok(m_func);
    }

    let m_entry_block = m_func.get_or_create_entry_block(ctx);
    let m_func_reg = m_func.get_region(ctx).unwrap();
    cctx.reset_for_region(m_entry_block);

    let blocks = rpo(function);
    // Map entry block
    let mut blocks_iter = blocks.iter();
    let Some(entry) = blocks_iter.next() else {
        return Ok(m_func);
    };

    cctx.block_map.insert(*entry, m_entry_block);
    {
        let val_map = &mut cctx.value_map;
        let m_entry_block_ref = m_entry_block.deref(ctx);
        // Map function args to entry block args.
        for (arg_idx, arg) in param_iter(function).enumerate() {
            val_map.insert(arg, m_entry_block_ref.get_argument(arg_idx));
        }
    }

    // Create, place and map rest of the blocks.
    for block in blocks_iter {
        let label = llvm_get_basic_block_name(*block)
            .filter(|name| !name.is_empty())
            .map(|name| cctx.id_legaliser.legalise(&name));
        let m_block = BasicBlock::new(ctx, label, vec![]);
        m_block.insert_at_back(m_func_reg, ctx);
        cctx.block_map.insert(*block, m_block);
    }

    // Finally, convert all blocks
    for block in blocks {
        let m_block = *cctx
            .block_map
            .get(&block)
            .expect("We have an unmapped block !");
        convert_block(ctx, cctx, block, m_block)?;
    }

    Ok(m_func)
}

fn convert_global(
    ctx: &mut Context,
    cctx: &mut ConversionContext,
    global: LLVMValue,
) -> Result<GlobalOp> {
    let llvm_name = llvm_get_value_name(global).unwrap_or_default();
    let name = cctx.id_legaliser.legalise(&llvm_name);

    let ty = convert_type(
        ctx,
        &mut ConversionContext::default(),
        llvm_global_get_value_type(global),
    )?;

    let op = GlobalOp::new(ctx, name.clone(), ty);

    let addr_space = llvm_get_pointer_address_space(llvm_type_of(global));
    if addr_space != 0 {
        op.set_address_space(ctx, addr_space);
    }

    if <Identifier as Into<String>>::into(name) != llvm_name {
        op.set_llvm_symbol_name(ctx, llvm_name);
    }

    let linkage = convert_linkage(llvm_get_linkage(global));
    op.set_attr_llvm_global_linkage(ctx, linkage);

    let alignment = llvm_get_alignment(global);
    if alignment != 0 {
        op.set_alignment(ctx, alignment);
    }

    if let Some(init) = llvm_get_initializer(global) {
        assert!(!llvm_is_declaration(global));
        // TODO: Use attribute based initializer for simple constants.
        let init_region = op.add_initializer_region(ctx);
        let entry_block = init_region.deref(ctx).iter(ctx).next().unwrap();
        cctx.reset_for_region(entry_block);

        // Convert the initializer.
        process_constant(ctx, cctx, init)?;
        let Some(m_val) = cctx.value_map.get(&init) else {
            panic!("We just processed this constant, it must be in the map");
        };

        let return_op = ReturnOp::new(ctx, Some(*m_val));
        return_op.get_operation().insert_at_back(entry_block, ctx);
    }

    Ok(op)
}

/// Convert [LLVMModule] to [ModuleOp].
pub fn convert_module(ctx: &mut Context, module: &LLVMModule) -> Result<ModuleOp> {
    let cctx = &mut ConversionContext::default();

    let module_name = llvm_get_module_identifier(module).unwrap_or_default();
    let module_name = cctx.id_legaliser.legalise(&module_name);

    let m = ModuleOp::new(ctx, module_name);

    // Convert globals.
    for gv in global_iter(module) {
        let m_gv = convert_global(ctx, cctx, gv)?;
        m.append_operation(ctx, m_gv.get_operation(), 0);
    }

    // Convert functions.
    for fun in function_iter(module) {
        let llvm_name = llvm_get_value_name(fun).expect("Expected function to have a name");
        if llvm_lookup_intrinsic_id(&llvm_name).is_some() {
            // Skip LLVM intrinsics.
            continue;
        }
        let m_fun = convert_function(ctx, cctx, fun)?;
        m.append_operation(ctx, m_fun.get_operation(), 0);
    }
    Ok(m)
}
