// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Wrapper LLVM functions to get the type sizes and alignment for a target.

use pliron::{
    arg_error_noloc, builtin::ops::ModuleOp, context::Context, result::Result, r#type::TypeHandle,
};

use crate::{
    attributes::get_data_layout,
    llvm_sys::{
        core::{LLVMContext, LLVMType},
        target::LLVMTargetData,
    },
    to_llvm_ir::{TypeConversionContext, convert_type},
};

#[derive(Debug, thiserror::Error)]
pub enum DataLayoutErr {
    #[error("Cannot get the data layout of the host: {0}")]
    NoHostLayout(String),
}

/// Target specific data layout provided by LLVM.
pub struct DataLayout {
    llvm_ctx: LLVMContext,
    target_data: LLVMTargetData,
    types: TypeConversionContext,
}

impl DataLayout {
    /// Build [DataLayout] as described by `layout`.
    pub fn new(layout: &str) -> Self {
        Self {
            llvm_ctx: LLVMContext::default(),
            target_data: LLVMTargetData::new(layout),
            types: TypeConversionContext::default(),
        }
    }

    /// Build [DataLayout] of *this* (the host) machine.
    pub fn host() -> Result<Self> {
        let target_data = LLVMTargetData::host()
            .map_err(|err| arg_error_noloc!(DataLayoutErr::NoHostLayout(err)))?;
        Ok(Self {
            llvm_ctx: LLVMContext::default(),
            target_data,
            types: TypeConversionContext::default(),
        })
    }

    /// Build [DataLayout] based on a `module`'s layout, if it has one.
    /// Falls back to [Self::host] if `module` doesn't have a layout set.
    pub fn from_module_layout(ctx: &Context, module: ModuleOp) -> Result<Self> {
        match get_data_layout(ctx, module) {
            Some(layout) if !layout.is_empty() => Ok(Self::new(&layout)),
            _ => Self::host(),
        }
    }

    /// The data layout string of this layout.
    pub fn string_representation(&self) -> String {
        self.target_data.copy_string_rep_of_target_data()
    }

    /// The number of bits that `ty` holds.
    pub fn type_size_in_bits(&mut self, ctx: &Context, ty: TypeHandle) -> Result<u64> {
        let ty = self.llvm_type(ctx, ty)?;
        Ok(self.target_data.size_of_type_in_bits(ty))
    }

    /// The number of bytes that the data of `ty` uses.
    pub fn type_store_size(&mut self, ctx: &Context, ty: TypeHandle) -> Result<u64> {
        let ty = self.llvm_type(ctx, ty)?;
        Ok(self.target_data.store_size_of_type(ty))
    }

    /// The number of bytes that one element of an array of `ty` uses.
    pub fn type_alloc_size(&mut self, ctx: &Context, ty: TypeHandle) -> Result<u64> {
        let ty = self.llvm_type(ctx, ty)?;
        Ok(self.target_data.abi_size_of_type(ty))
    }

    /// The alignment in bytes of `ty`, provided by the ABI.
    pub fn abi_type_align(&mut self, ctx: &Context, ty: TypeHandle) -> Result<u32> {
        let ty = self.llvm_type(ctx, ty)?;
        Ok(self.target_data.abi_alignment_of_type(ty))
    }

    /// Does an array of `ty` hold its elements with no padding between them?
    ///
    /// This is equivalent to [Self::type_store_size] == [Self::type_alloc_size]
    pub fn packs_exactly(&mut self, ctx: &Context, ty: TypeHandle) -> Result<bool> {
        let ty = self.llvm_type(ctx, ty)?;
        Ok(self.target_data.store_size_of_type(ty) == self.target_data.abi_size_of_type(ty))
    }

    /// Get the LLVM type of `ty`, building it if the cache does not hold it.
    fn llvm_type(&mut self, ctx: &Context, ty: TypeHandle) -> Result<LLVMType> {
        convert_type(ctx, &self.llvm_ctx, &mut self.types, ty)
    }
}

#[cfg(test)]
mod tests {
    use pliron::{
        builtin::types::{FP16Type, FP64Type, IntegerType, Signedness},
        context::Context,
        result::ExpectOk,
    };

    use crate::{data_layout::DataLayout, types::ArrayType};

    /// A layout of x86-64, so that the answers do not depend on the host.
    const X86_64: &str =
        "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128";

    #[test]
    fn sizes_of_elements() {
        let ctx = &mut Context::new();
        let mut layout = DataLayout::new(X86_64);

        // An i24 stores in 3 bytes, but an array gives it 4.
        let i24 = IntegerType::get(ctx, 24, Signedness::Signless).into();
        assert_eq!(layout.type_size_in_bits(ctx, i24).expect_ok(ctx), 24);
        assert_eq!(layout.type_store_size(ctx, i24).expect_ok(ctx), 3);
        assert_eq!(layout.type_alloc_size(ctx, i24).expect_ok(ctx), 4);
        assert_eq!(layout.abi_type_align(ctx, i24).expect_ok(ctx), 4);
        assert!(!layout.packs_exactly(ctx, i24).expect_ok(ctx));

        // The common widths have no padding.
        for ty in [
            IntegerType::get(ctx, 8, Signedness::Signless).into(),
            IntegerType::get(ctx, 32, Signedness::Signless).into(),
            IntegerType::get(ctx, 128, Signedness::Signless).into(),
            FP16Type::get(ctx).into(),
            FP64Type::get(ctx).into(),
        ] {
            assert!(layout.packs_exactly(ctx, ty).expect_ok(ctx));
            assert_eq!(
                layout.type_store_size(ctx, ty).expect_ok(ctx),
                layout.type_alloc_size(ctx, ty).expect_ok(ctx)
            );
        }
    }

    #[test]
    fn size_of_array_holds_the_padding() {
        let ctx = &mut Context::new();
        let mut layout = DataLayout::new(X86_64);

        // Three i24 elements of 4 bytes each, and not the 9 bytes of their data.
        let i24 = IntegerType::get(ctx, 24, Signedness::Signless);
        let array = ArrayType::get(ctx, i24.into(), 3).into();
        assert_eq!(layout.type_store_size(ctx, array).expect_ok(ctx), 12);
        assert_eq!(layout.type_alloc_size(ctx, array).expect_ok(ctx), 12);
    }

    #[test]
    fn host_layout_is_available() {
        let ctx = &mut Context::new();
        let mut layout = DataLayout::host().expect_ok(ctx);
        assert!(!layout.string_representation().is_empty());

        // A pointer is as large as the host's pointer.
        let ptr = crate::types::PointerType::get(ctx, 0).into();
        assert_eq!(
            layout.type_store_size(ctx, ptr).expect_ok(ctx) as usize,
            size_of::<*const u8>()
        );
    }
}
