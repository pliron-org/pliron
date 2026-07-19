// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Builtin dialect: [Op](crate::op::Op)s, [Type](crate::type::Type)s and [Attribute](crate::attribute::Attribute)s

pub mod attr_interfaces;
pub mod attributes;
pub mod interface_impls;
pub mod op_interfaces;
pub mod ops;
pub mod type_interfaces;
pub mod types;

crate::dict_key!(
    /// Key for debug info related attributes.
    ATTR_KEY_DEBUG_INFO, "builtin_debug_info"
);
