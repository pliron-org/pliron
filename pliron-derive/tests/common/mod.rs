// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

use pliron::builtin::op_interfaces::IsTerminatorInterface;
use pliron_derive::{def_op, derive_op_interface_impl, format_op, verify_succ};

#[def_op("test.return")]
#[format_op("$0")]
#[derive_op_interface_impl(IsTerminatorInterface)]
#[verify_succ]
pub struct ReturnOp;
