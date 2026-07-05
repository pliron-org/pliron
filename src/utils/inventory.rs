//! Helper for integration for inventory

/// A wrapper around [LazyLock] to allow its use with [inventory](crate::inventory).
/// "This collect! call must be in the same crate that defines the plugin type."
pub struct InventoryWrapper<T: 'static>(pub &'static T);
