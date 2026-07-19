// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Implementation of various interfaces for builtin dialect entities.

use alloc::{vec, vec::Vec};
use pliron::{
    attribute::AttrObj,
    context::Context,
    derive::op_interface_impl,
    irbuild::{IRStatus, rewriter::Rewriter},
    opts::constants::ConstFoldInterface,
};

use crate::{builtin::ops::ConstantOp, opts::dce::SideEffects};

#[op_interface_impl]
impl ConstFoldInterface for ConstantOp {
    fn check_fold(
        &self,
        ctx: &Context,
        _operand_attrs: &[Option<AttrObj>],
    ) -> Vec<Option<AttrObj>> {
        vec![Some(self.get_value(ctx))]
    }

    fn fold_in_place(
        &self,
        _ctx: &mut Context,
        _operand_attrs: &[Option<AttrObj>],
        _rewriter: &mut dyn Rewriter,
    ) -> IRStatus {
        IRStatus::Unchanged
    }
}

#[op_interface_impl]
impl SideEffects for ConstantOp {
    fn has_side_effects(&self, _ctx: &Context) -> bool {
        false
    }
}
