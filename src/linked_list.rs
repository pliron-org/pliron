//! Provide linked-list operations for `Ptr<T: LinkedList>`.

use crate::context::{Context, Ptr, private::ArenaObj};

/// The setter methods on [LinkedList] and [ContainsLinkedList]
/// are not safe for using from anywhere except the impls here.
/// But Rust doesn't allow private trait functions. Hence this
/// [workaround](https://jack.wrenn.fyi/blog/private-trait-methods/)
pub(crate) mod private {
    use crate::context::{Ptr, private::ArenaObj};

    pub trait ContainsLinkedList<T: LinkedList> {
        /// Simply set the head pointer.
        fn set_head(&mut self, head: Option<Ptr<T>>);
        /// Simply set the tail pointer
        fn set_tail(&mut self, tail: Option<Ptr<T>>);
    }

    pub trait LinkedList: ArenaObj + PartialEq {
        type ContainerType: super::ContainsLinkedList<Self> + ArenaObj
        where
            Self: super::LinkedList;
        /// Simply set the item next to this in the list.
        fn set_next(&mut self, next: Option<Ptr<Self>>);
        /// Simply set the item previous to this in the list.
        fn set_prev(&mut self, prev: Option<Ptr<Self>>);
        /// Set the container for this node.
        fn set_container(&mut self, container: Option<Ptr<Self::ContainerType>>)
        where
            Self: super::LinkedList;
    }
}

/// An object that contains a linked list.
pub trait ContainsLinkedList<T: LinkedList>: private::ContainsLinkedList<T> {
    /// Simply get the head of the list.
    fn get_head(&self) -> Option<Ptr<T>>;
    /// Simply get the tail of the list.
    fn get_tail(&self) -> Option<Ptr<T>>;
    /// Get an iterator over the items. Context is borrowed throughout.
    fn iter<'a>(&self, ctx: &'a Context) -> Iter<'a, T> {
        Iter {
            next: self.get_head(),
            next_back: self.get_tail(),
            ctx,
        }
    }
}

/// An iterator over the elements of a [LinkedList]
pub struct Iter<'a, T: LinkedList> {
    next: Option<Ptr<T>>,
    next_back: Option<Ptr<T>>,
    ctx: &'a Context,
}

impl<T: LinkedList> Clone for Iter<'_, T> {
    fn clone(&self) -> Self {
        Self {
            next: self.next,
            next_back: self.next_back,
            ctx: self.ctx,
        }
    }
}

impl<T: LinkedList> Iterator for Iter<'_, T> {
    type Item = Ptr<T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next.inspect(|curr| {
            if *curr
                == self
                    .next_back
                    .expect("Some(next) => Some(next_back) violated")
            {
                self.next = None;
                self.next_back = None;
            } else {
                self.next = curr.deref(self.ctx).get_next();
            }
        })
    }

    fn last(mut self) -> Option<Self::Item> {
        self.next_back()
    }
}

impl<T: LinkedList> DoubleEndedIterator for Iter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.next_back.inspect(|curr| {
            if *curr == self.next.expect("Some(next_back) => Some(next) violated") {
                self.next_back = None;
                self.next = None;
            } else {
                self.next_back = curr.deref(self.ctx).get_prev();
            }
        })
    }
}

/// Implements a linked list based on [Ptr]
/// Types implementing this trait must provide simple
/// getters and setters for prev and next fields.
/// Actual linked list operations are implemented for `Ptr<T: LinkedList>`
pub trait LinkedList: private::LinkedList {
    /// Simple getter for the item previous to this in the list.
    fn get_next(&self) -> Option<Ptr<Self>>;
    /// Simple getter for the item previous to this in the list.
    fn get_prev(&self) -> Option<Ptr<Self>>;
    /// Get a reference to the object that contains this linked list.
    fn get_container(&self) -> Option<Ptr<Self::ContainerType>>;
}

/// Append all nodes from `donor` to the back of `donee`, leaving `donor` empty.
pub fn append_linked_list<C: ContainsLinkedList<U> + ArenaObj, U: LinkedList<ContainerType = C>>(
    donee: Ptr<C>,
    donor: Ptr<C>,
    ctx: &Context,
) {
    let mut current = donor.deref(ctx).get_head();
    while let Some(node) = current {
        let next = node.deref(ctx).get_next();
        node.unlink(ctx);
        node.insert_at_back(donee, ctx);
        current = next;
    }
}

/// Linked list operations on Ptr<T: LinkedList> for convenience and safety.
impl<T> Ptr<T>
where
    T: LinkedList,
{
    /// Insert self after mark.
    pub fn insert_after(&self, ctx: &Context, mark: Ptr<T>) {
        {
            let node = self.deref(ctx);
            assert!(
                node.get_prev().is_none()
                    && node.get_next().is_none()
                    && node.get_container().is_none(),
                "LinkedList node must be unlinked before relinking"
            );
            let mark = mark.deref(ctx);
            assert!(
                mark.get_container().is_some(),
                "insert_after: Mark node itself is unlinked"
            );
        }
        let next;
        let container;
        // If mark == *self, we don't want two deref_mut() on it.
        {
            let mut mark_ref = mark.deref_mut(ctx);
            container = mark_ref.get_container().unwrap();
            next = mark_ref.get_next();
            match next {
                Some(next) => {
                    assert!(next.deref(ctx).get_prev().unwrap() == mark);
                    next.deref_mut(ctx).set_prev(Some(*self));
                }
                None => {
                    assert!(container.deref(ctx).get_tail().unwrap() == mark);
                    private::ContainsLinkedList::set_tail(
                        &mut (*container.deref_mut(ctx)),
                        Some(*self),
                    );
                }
            }
            (*mark_ref).set_next(Some(*self));
        }

        let mut node = self.deref_mut(ctx);
        node.set_next(next);
        node.set_prev(Some(mark));
        node.set_container(Some(container));
    }

    /// Insert self before mark.
    pub fn insert_before(&self, ctx: &Context, mark: Ptr<T>) {
        {
            let node = self.deref(ctx);
            assert!(
                node.get_prev().is_none()
                    && node.get_next().is_none()
                    && node.get_container().is_none(),
                "LinkedList node must be unlinked before relinking"
            );
            let mark = mark.deref(ctx);
            assert!(
                mark.get_container().is_some(),
                "insert_before: Mark node itself is unlinked"
            );
        }

        let container;
        let prev;
        // If mark == *self, we don't want two deref_mut() on it.
        {
            let mut mark_ref = mark.deref_mut(ctx);
            container = mark_ref.get_container().unwrap();
            prev = mark_ref.get_prev();
            match prev {
                Some(prev) => {
                    assert!(prev.deref(ctx).get_next().unwrap() == mark);
                    prev.deref_mut(ctx).set_next(Some(*self));
                }
                None => {
                    assert!(container.deref(ctx).get_head().unwrap() == mark);
                    private::ContainsLinkedList::set_head(
                        &mut (*container.deref_mut(ctx)),
                        Some(*self),
                    );
                }
            }
            (*mark_ref).set_prev(Some(*self));
        }

        let mut node = self.deref_mut(ctx);
        node.set_prev(prev);
        node.set_next(Some(mark));
        node.set_container(Some(container));
    }

    /// Insert self as the head of the list.
    pub fn insert_at_front(&self, container: Ptr<T::ContainerType>, ctx: &Context) {
        let mut node = self.deref_mut(ctx);
        assert!(
            node.get_prev().is_none()
                && node.get_next().is_none()
                && node.get_container().is_none(),
            "LinkedList node must be unlinked before relinking"
        );
        let mut container_ref = container.deref_mut(ctx);
        let head = container_ref.get_head();
        match head {
            Some(head) => {
                assert!(head.deref(ctx).get_prev().is_none());
                head.deref_mut(ctx).set_prev(Some(*self))
            }
            None => {
                private::ContainsLinkedList::set_tail(&mut (*container_ref), Some(*self));
            }
        }
        node.set_next(head);
        private::ContainsLinkedList::set_head(&mut (*container_ref), Some(*self));
        node.set_container(Some(container));
    }

    /// Insert self as the tail of the list.
    pub fn insert_at_back(&self, container: Ptr<T::ContainerType>, ctx: &Context) {
        let mut node = self.deref_mut(ctx);
        assert!(
            node.get_prev().is_none()
                && node.get_next().is_none()
                && node.get_container().is_none(),
            "LinkedList node must be unlinked before relinking"
        );
        let mut container_ref = container.deref_mut(ctx);
        let tail = container_ref.get_tail();
        match tail {
            Some(tail) => {
                assert!(tail.deref(ctx).get_next().is_none());
                tail.deref_mut(ctx).set_next(Some(*self));
            }
            None => {
                private::ContainsLinkedList::set_head(&mut (*container_ref), Some(*self));
            }
        }
        node.set_prev(tail);
        private::ContainsLinkedList::set_tail(&mut (*container_ref), Some(*self));
        node.set_container(Some(container));
    }

    /// Is this node part of a linked list?
    pub fn is_linked(&self, ctx: &Context) -> bool {
        let node = &*self.deref(ctx);
        let has_container = node.get_container().is_some();
        assert!(
            has_container || node.get_next().is_none() && node.get_prev().is_none(),
            "LinkedList node has no container, but has next/prev node"
        );
        has_container
    }

    /// Unlink self from list.
    pub fn unlink(&self, ctx: &Context) {
        let container = self
            .deref(ctx)
            .get_container()
            .expect("LinkedList: Attempt to remove unlinked node");
        let next = self.deref(ctx).get_next();
        let prev = self.deref(ctx).get_prev();
        match next {
            Some(next) => next.deref_mut(ctx).set_prev(prev),
            None => {
                private::ContainsLinkedList::set_tail(&mut (*container.deref_mut(ctx)), prev);
            }
        }
        match prev {
            Some(prev) => {
                prev.deref_mut(ctx).set_next(next);
            }
            None => {
                private::ContainsLinkedList::set_head(&mut (*container.deref_mut(ctx)), next);
            }
        }

        let mut node = self.deref_mut(ctx);
        node.set_next(None);
        node.set_prev(None);
        node.set_container(None);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use core::cell::OnceCell;
    use std::{boxed::Box, thread_local};

    use super::*;
    use crate::context::{Arena, AuxDataIndex, Context, Ptr, private::ArenaObj};
    use alloc::{vec, vec::Vec};

    #[derive(PartialEq)]
    struct LLNode {
        data: u64,
        next: Option<Ptr<LLNode>>,
        prev: Option<Ptr<LLNode>>,
        parent: Option<Ptr<LLRoot>>,
        self_ptr: Ptr<LLNode>,
    }

    thread_local! {
        static TEST_ARENA_INDEX: OnceCell<AuxDataIndex> = const { OnceCell::new() };
    }

    impl ArenaObj for LLNode {
        fn get_arena(ctx: &Context) -> &Arena<Self> {
            &ctx.aux_data[TEST_ARENA_INDEX.with(|cell| *cell.get().expect("ARENA_INDEX not set"))]
                .downcast_ref::<LinkedListTestArena>()
                .expect("linked_list_store is not LinkedListTestArena")
                .nodes
        }

        fn get_arena_mut(ctx: &mut Context) -> &mut Arena<Self> {
            &mut ctx.aux_data
                [TEST_ARENA_INDEX.with(|cell| *cell.get().expect("ARENA_INDEX not set"))]
            .downcast_mut::<LinkedListTestArena>()
            .expect("linked_list_store is not LinkedListTestArena")
            .nodes
        }

        fn get_self_ptr(&self, _ctx: &Context) -> Ptr<Self> {
            self.self_ptr
        }

        fn dealloc_sub_objects(_ptr: Ptr<Self>, _ctx: &mut Context) {}
    }

    impl LLNode {
        pub fn new(ctx: &mut Context, data: u64) -> Ptr<LLNode> {
            let f = |self_ptr: Ptr<LLNode>| LLNode {
                self_ptr,
                data,
                next: None,
                prev: None,
                parent: None,
            };
            Self::alloc(ctx, f)
        }
    }

    impl private::LinkedList for LLNode {
        type ContainerType = LLRoot;

        fn set_next(&mut self, next: Option<Ptr<Self>>) {
            self.next = next;
        }

        fn set_prev(&mut self, prev: Option<Ptr<Self>>) {
            self.prev = prev;
        }

        fn set_container(&mut self, container: Option<Ptr<Self::ContainerType>>) {
            self.parent = container;
        }
    }

    impl LinkedList for LLNode {
        fn get_next(&self) -> Option<Ptr<Self>> {
            self.next
        }

        fn get_prev(&self) -> Option<Ptr<Self>> {
            self.prev
        }

        fn get_container(&self) -> Option<Ptr<Self::ContainerType>> {
            self.parent
        }
    }

    struct LLRoot {
        first: Option<Ptr<LLNode>>,
        last: Option<Ptr<LLNode>>,
        self_ptr: Ptr<LLRoot>,
    }

    impl LLRoot {
        pub fn empty(ctx: &mut Context) -> Ptr<LLRoot> {
            let f = |self_ptr: Ptr<LLRoot>| LLRoot {
                self_ptr,
                first: None,
                last: None,
            };
            Self::alloc(ctx, f)
        }
    }

    impl ArenaObj for LLRoot {
        fn get_arena(ctx: &Context) -> &Arena<Self> {
            &ctx.aux_data[TEST_ARENA_INDEX.with(|cell| *cell.get().expect("ARENA_INDEX not set"))]
                .downcast_ref::<LinkedListTestArena>()
                .expect("linked_list_store is not LinkedListTestArena")
                .containers
        }

        fn get_arena_mut(ctx: &mut Context) -> &mut Arena<Self> {
            &mut ctx.aux_data
                [TEST_ARENA_INDEX.with(|cell| *cell.get().expect("ARENA_INDEX not set"))]
            .downcast_mut::<LinkedListTestArena>()
            .expect("linked_list_store is not LinkedListTestArena")
            .containers
        }

        fn get_self_ptr(&self, _ctx: &Context) -> Ptr<Self> {
            self.self_ptr
        }

        fn dealloc_sub_objects(_ptr: Ptr<Self>, _ctx: &mut Context) {}
    }

    impl ContainsLinkedList<LLNode> for LLRoot {
        fn get_head(&self) -> Option<Ptr<LLNode>> {
            self.first
        }

        fn get_tail(&self) -> Option<Ptr<LLNode>> {
            self.last
        }
    }

    impl private::ContainsLinkedList<LLNode> for LLRoot {
        fn set_head(&mut self, head: Option<Ptr<LLNode>>) {
            self.first = head;
        }

        fn set_tail(&mut self, tail: Option<Ptr<LLNode>>) {
            self.last = tail;
        }
    }

    #[derive(Default)]
    pub(crate) struct LinkedListTestArena {
        nodes: Arena<LLNode>,
        containers: Arena<LLRoot>,
    }

    fn validate_list(ctx: &Context, root: Ptr<LLRoot>, gold: Vec<u64>) {
        let root_vec: Vec<_> = root
            .deref(ctx)
            .iter(ctx)
            .map(|n| n.deref(ctx).data)
            .collect();
        assert!(
            root_vec == gold,
            "\nExpected: {:?}\nvs\nFound: {:?}",
            gold,
            root_vec
        );
        // Compare reverses, to test the reverse iterator.
        let root_rev_vec: Vec<_> = root
            .deref(ctx)
            .iter(ctx)
            .rev()
            .map(|n| n.deref(ctx).data)
            .collect();
        let gold_rev: Vec<_> = gold.iter().cloned().rev().collect();
        assert!(
            root_rev_vec == gold_rev,
            "\nExpected: {:?}\nvs\nFound: {:?}",
            gold_rev,
            root_rev_vec,
        );
    }

    fn add_arena_to_context(ctx: &mut Context) {
        let arena = LinkedListTestArena::default();
        let index = ctx.aux_data.insert(Box::new(arena));
        TEST_ARENA_INDEX.with(|cell| {
            cell.set(index)
                .expect("ARENA_INDEX should not be set more than once");
        });
    }

    #[test]
    fn success() {
        let ctx = &mut Context::default();
        add_arena_to_context(ctx);
        let root = LLRoot::empty(ctx);

        let n1 = LLNode::new(ctx, 1);
        let n2 = LLNode::new(ctx, 2);
        let n3 = LLNode::new(ctx, 3);
        let n4 = LLNode::new(ctx, 4);

        assert!(
            !n1.is_linked(ctx) && !n2.is_linked(ctx) && !n3.is_linked(ctx) && !n4.is_linked(ctx)
        );

        n1.insert_at_front(root, ctx);
        validate_list(ctx, root, vec![1]);

        assert!(
            n1.is_linked(ctx) && !n2.is_linked(ctx) && !n3.is_linked(ctx) && !n4.is_linked(ctx)
        );

        n2.insert_after(ctx, n1);
        validate_list(ctx, root, vec![1, 2]);

        assert!(n1.is_linked(ctx) && n2.is_linked(ctx) && !n3.is_linked(ctx) && !n4.is_linked(ctx));

        n4.insert_after(ctx, n2);
        validate_list(ctx, root, vec![1, 2, 4]);

        assert!(n1.is_linked(ctx) && n2.is_linked(ctx) && !n3.is_linked(ctx) && n4.is_linked(ctx));

        n2.unlink(ctx);
        validate_list(ctx, root, vec![1, 4]);
        n1.unlink(ctx);
        validate_list(ctx, root, vec![4]);
        n4.unlink(ctx);
        validate_list(ctx, root, vec![]);

        n1.insert_at_back(root, ctx);
        validate_list(ctx, root, vec![1]);
        n2.insert_at_back(root, ctx);
        validate_list(ctx, root, vec![1, 2]);

        n1.unlink(ctx);
        validate_list(ctx, root, vec![2]);
        assert!(!n1.is_linked(ctx) && n2.is_linked(ctx) && !n3.is_linked(ctx));
        n3.insert_before(ctx, n2);
        validate_list(ctx, root, vec![3, 2]);
        assert!(!n1.is_linked(ctx) && n2.is_linked(ctx) && n3.is_linked(ctx));
    }

    #[test]
    #[should_panic(expected = "must be unlinked before relinking")]
    fn reinsert_panic() {
        let ctx = &mut Context::default();
        add_arena_to_context(ctx);
        let root = LLRoot::empty(ctx);

        let n1 = LLNode::new(ctx, 1);
        n1.insert_at_front(root, ctx);
        // Reinserting an exiting node must cause panic.
        n1.insert_at_front(root, ctx);
    }

    #[test]
    #[should_panic(expected = "Attempt to remove unlinked node")]
    fn uninserted_remove_panic() {
        let ctx = &mut Context::default();
        add_arena_to_context(ctx);
        let n1 = LLNode::new(ctx, 1);
        // Removing an unlinked node must cause panic.
        n1.unlink(ctx);
    }

    #[test]
    #[should_panic(expected = "Attempt to remove unlinked node")]
    fn reremove_panic() {
        let ctx = &mut Context::default();
        add_arena_to_context(ctx);
        let root = LLRoot::empty(ctx);

        let n1 = LLNode::new(ctx, 1);
        n1.insert_at_front(root, ctx);
        n1.unlink(ctx);
        // Removing an unlinked node must cause panic.
        n1.unlink(ctx);
    }

    #[test]
    #[should_panic(expected = " Mark node itself is unlinked")]
    fn insert_after_unlinked_panic() {
        let ctx = &mut Context::default();
        add_arena_to_context(ctx);

        let n1 = LLNode::new(ctx, 1);
        let n2 = LLNode::new(ctx, 2);
        // n1 itself is unlinked, so this is a panic.
        n2.insert_after(ctx, n1);
    }

    #[test]
    #[should_panic(expected = " Mark node itself is unlinked")]
    fn insert_before_unlinked_panic() {
        let ctx = &mut Context::default();
        add_arena_to_context(ctx);

        let n1 = LLNode::new(ctx, 1);
        let n2 = LLNode::new(ctx, 2);
        // n1 itself is unlinked, so this is a panic.
        n2.insert_before(ctx, n1);
    }

    #[test]
    fn test_clear_linked_list() {
        let ctx = &mut Context::default();
        add_arena_to_context(ctx);
        let root = LLRoot::empty(ctx);

        let n1 = LLNode::new(ctx, 1);
        let n2 = LLNode::new(ctx, 2);
        let n3 = LLNode::new(ctx, 3);

        n1.insert_at_front(root, ctx);
        n2.insert_at_back(root, ctx);
        n3.insert_at_back(root, ctx);
        validate_list(ctx, root, vec![1, 2, 3]);
    }

    #[test]
    fn test_append_linked_list() {
        let ctx = &mut Context::default();
        add_arena_to_context(ctx);
        let root1 = LLRoot::empty(ctx);
        let root2 = LLRoot::empty(ctx);

        let n1 = LLNode::new(ctx, 1);
        let n2 = LLNode::new(ctx, 2);
        let n3 = LLNode::new(ctx, 3);
        let n4 = LLNode::new(ctx, 4);

        n1.insert_at_back(root1, ctx);
        n2.insert_at_back(root1, ctx);
        n3.insert_at_back(root2, ctx);
        n4.insert_at_back(root2, ctx);

        validate_list(ctx, root1, vec![1, 2]);
        validate_list(ctx, root2, vec![3, 4]);

        super::append_linked_list(root1, root2, ctx);

        validate_list(ctx, root1, vec![1, 2, 3, 4]);
        validate_list(ctx, root2, vec![]);

        assert!(n1.is_linked(ctx) && n2.is_linked(ctx) && n3.is_linked(ctx) && n4.is_linked(ctx));
    }

    #[test]
    fn test_append_empty_to_nonempty() {
        let ctx = &mut Context::default();
        add_arena_to_context(ctx);
        let root1 = LLRoot::empty(ctx);
        let root2 = LLRoot::empty(ctx);

        let n1 = LLNode::new(ctx, 1);
        n1.insert_at_back(root1, ctx);

        super::append_linked_list(root1, root2, ctx);
        validate_list(ctx, root1, vec![1]);
        validate_list(ctx, root2, vec![]);
    }

    #[test]
    fn test_append_nonempty_to_empty() {
        let ctx = &mut Context::default();
        add_arena_to_context(ctx);
        let root1 = LLRoot::empty(ctx);
        let root2 = LLRoot::empty(ctx);

        let n1 = LLNode::new(ctx, 1);
        n1.insert_at_back(root2, ctx);

        super::append_linked_list(root1, root2, ctx);
        validate_list(ctx, root1, vec![1]);
        validate_list(ctx, root2, vec![]);
    }
}
