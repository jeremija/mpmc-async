use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Implements a double-linked list with O(1) complexity for:
/// - `push` operation, and
/// - `remove` of _any_ node
///
/// with the following caveat:
///
/// For every call to `push` `remove` MUST be called before the resulting `NodeRef` is dropped,
/// or else dropping `NodeRef` will panic.
pub struct LinkedList<T> {
    list_ptr: *mut u8,
    head_tail: HeadTail<T>,
    len: usize,
}

unsafe impl<T> Send for LinkedList<T> where T: Send {}
unsafe impl<T> Sync for LinkedList<T> where T: Sync {}

impl<T> Default for LinkedList<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LinkedList<T> {
    pub fn new() -> Self {
        Self {
            list_ptr: Box::into_raw(Box::new(0)),
            head_tail: HeadTail {
                head: std::ptr::null_mut(),
                tail: std::ptr::null_mut(),
            },
            len: 0,
        }
    }

    /// Returns the total number of items in the list.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Pushes the value to the head of the list. The resulting [NodeRef] can be used to
    /// remove the entry or add items to either side of it.
    ///
    /// Complexity: O(1)
    pub fn push_head(&mut self, value: T) -> NodeRef<T> {
        self.len += 1;

        let mut node = Node::new(value);

        assert_eq!(
            self.head_tail.head.is_null(),
            self.head_tail.tail.is_null(),
            "head and tail should both be null or non-null"
        );

        let head = unsafe { self.head_tail.head.as_mut() };

        match head {
            None => {
                let node_ptr = Box::into_raw(Box::new(node));
                self.head_tail.head = node_ptr;
                self.head_tail.tail = node_ptr;
                NodeRef::new(self.list_ptr, node_ptr)
            }
            Some(head) => {
                node.next = self.head_tail.head;
                let node_ptr = Box::into_raw(Box::new(node));
                head.prev = node_ptr;
                self.head_tail.head = node_ptr;
                NodeRef::new(self.list_ptr, node_ptr)
            }
        }
    }

    /// Pushes the value to the tail of the list. The resulting [NodeRef] can be used to
    /// remove the entry or add items to either side of it.
    ///
    /// Complexity: O(1)
    pub fn push_tail(&mut self, value: T) -> NodeRef<T> {
        self.len += 1;

        let mut node = Node::new(value);

        assert_eq!(
            self.head_tail.head.is_null(),
            self.head_tail.tail.is_null(),
            "head and tail should both be null or non-null"
        );

        let tail = unsafe { self.head_tail.tail.as_mut() };

        match tail {
            None => {
                let node_ptr = Box::into_raw(Box::new(node));
                self.head_tail.head = node_ptr;
                self.head_tail.tail = node_ptr;
                NodeRef::new(self.list_ptr, node_ptr)
            }
            Some(tail) => {
                node.prev = self.head_tail.tail;
                let node_ptr = Box::into_raw(Box::new(node));
                tail.next = node_ptr;
                self.head_tail.tail = node_ptr;
                NodeRef::new(self.list_ptr, node_ptr)
            }
        }
    }

    /// Pushes an element before the referenced item. In case the [NodeRef] is no longer a part
    /// of this list, it returns `Err(value)`.
    ///
    /// Complexity: O(1)
    pub fn push_before(&mut self, node_ref: &NodeRef<T>, value: T) -> Result<NodeRef<T>, T> {
        let Some(other_node) = self.get_node_mut(node_ref) else {
            return Err(value);
        };

        let mut node = Node::new(value);
        node.prev = other_node.prev;
        node.next = node_ref.node_ptr;

        let prev_ptr = node.prev;

        let node_ptr = Box::into_raw(Box::new(node));
        other_node.prev = node_ptr;

        let prev = unsafe { prev_ptr.as_mut() };

        if let Some(prev) = prev {
            prev.next = node_ptr;
        } else {
            self.head_tail.head = node_ptr;
        }

        self.len += 1;

        Ok(NodeRef::new(self.list_ptr, node_ptr))
    }

    /// Pushes an element after the referenced item. In case the [NodeRef] is no longer a part of
    /// this list, it returns `Err(value)`.
    ///
    /// Complexity: O(1)
    pub fn push_after(&mut self, node_ref: &NodeRef<T>, value: T) -> Result<NodeRef<T>, T> {
        let Some(other_node) = self.get_node_mut(node_ref) else {
            return Err(value);
        };

        let mut node = Node::new(value);
        node.prev = node_ref.node_ptr;
        node.next = other_node.next;

        let next_ptr = node.next;

        let node_ptr = Box::into_raw(Box::new(node));
        other_node.next = node_ptr;

        let next = unsafe { next_ptr.as_mut() };

        if let Some(next) = next {
            next.prev = node_ptr;
        } else {
            self.head_tail.tail = node_ptr;
        }

        self.len += 1;

        Ok(NodeRef::new(self.list_ptr, node_ptr))
    }

    /// Removes the first item in the list if any and returns it.
    ///
    /// Complexity: O(1)
    pub fn pop_head(&mut self) -> Option<T> {
        assert_eq!(
            self.head_tail.head.is_null(),
            self.head_tail.tail.is_null(),
            "head and tail should both be null or non-null"
        );

        if self.head_tail.head.is_null() {
            return None;
        }

        let node = unsafe { Box::from_raw(self.head_tail.head) };

        if self.head_tail.head == self.head_tail.tail {
            self.head_tail.head = std::ptr::null_mut();
            self.head_tail.tail = std::ptr::null_mut();
        } else {
            self.head_tail.head = node.next;

            if let Some(next) = unsafe { node.next.as_mut() } {
                next.prev = std::ptr::null_mut();
            }
        }

        Some(node.value)
    }

    /// Removes the last item in the list if any and returns it.
    ///
    /// Complexity: O(1)
    pub fn pop_tail(&mut self) -> Option<T> {
        assert_eq!(
            self.head_tail.head.is_null(),
            self.head_tail.tail.is_null(),
            "head and tail should both be null or non-null"
        );

        if self.head_tail.tail.is_null() {
            return None;
        }

        let node = unsafe { Box::from_raw(self.head_tail.tail) };

        if self.head_tail.head == self.head_tail.tail {
            self.head_tail.head = std::ptr::null_mut();
            self.head_tail.tail = std::ptr::null_mut();
        } else {
            self.head_tail.tail = node.prev;

            if let Some(prev) = unsafe { node.prev.as_mut() } {
                prev.next = std::ptr::null_mut();
            }
        }

        Some(node.value)
    }

    /// Gets a reference to the value by its ref.
    ///
    /// In case the item was removed from the list it returns [None].
    ///
    /// Complexity: O(1)
    pub fn get(&self, node_ref: &NodeRef<T>) -> Option<&T> {
        self.get_node(node_ref).map(|node| &node.value)
    }

    /// Gets a mutable reference to the value by its ref.
    ///
    /// In case the item was removed from the list it returns [None].
    ///
    /// Complexity: O(1)
    pub fn get_mut(&mut self, node_ref: &NodeRef<T>) -> Option<&mut T> {
        self.get_node_mut(node_ref).map(|node| &mut node.value)
    }

    fn get_node(&self, node_ref: &NodeRef<T>) -> Option<&Node<T>> {
        if node_ref.list_ptr != self.list_ptr {
            // This ref belongs to a different list instance or has been removed.
            return None;
        }

        let node = unsafe { node_ref.node_ptr.as_ref()? };

        if node.ref_count.load(Ordering::Relaxed) < 2 {
            // This node was removed from the list.
            None
        } else {
            Some(node)
        }
    }

    fn get_node_mut(&mut self, node_ref: &NodeRef<T>) -> Option<&mut Node<T>> {
        if node_ref.list_ptr != self.list_ptr {
            // This ref belongs to a different list instance or has been removed.
            return None;
        }

        let node = unsafe { node_ref.node_ptr.as_mut()? };

        if node.removed.load(Ordering::Relaxed) {
            // This node was removed from the list.
            None
        } else {
            Some(node)
        }
    }

    /// Returns a reference to the first element, if any.
    pub fn head(&self) -> Option<&T> {
        unsafe { self.head_tail.head.as_ref().map(|node| &node.value) }
    }

    /// Returns a reference to the last element, if any.
    pub fn tail(&self) -> Option<&T> {
        unsafe { self.head_tail.tail.as_ref().map(|node| &node.value) }
    }

    /// Returns an iterator.
    pub fn iter(&self) -> Iter<'_, T> {
        Iter::new(HeadTail {
            head: self.head_tail.head,
            tail: self.head_tail.tail,
        })
    }

    /// Returns an iterator.
    pub fn into_iter(self) -> IntoIter<T> {
        IntoIter::new(self)
    }

    /// Removes the element by reference.
    ///
    /// Returns [None] when the item is no longer part of the list.
    ///
    /// Complexity: O(1).
    pub fn remove(&mut self, mut node_ref: NodeRef<T>) -> Option<T> {
        if node_ref.list_ptr != self.list_ptr {
            return None;
        }

        let node_ptr = node_ref.node_ptr;

        let node = unsafe { Box::from_raw(node_ptr) };

        let value = node.remove(&mut self.head_tail);

        // Detach from the list.
        node_ref.list_ptr = std::ptr::null_mut();

        println!("setting node_ref.0 to null");

        drop(node_ref);

        self.len -= 1;

        Some(value)
    }
}

impl<T> FromIterator<T> for LinkedList<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut list = Self::new();

        for item in iter {
            list.push_tail(item);
        }

        list
    }
}

impl<'a, T> IntoIterator for &'a LinkedList<T> {
    type Item = &'a T;

    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T> IntoIterator for LinkedList<T> {
    type Item = T;

    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}

impl<T> Drop for LinkedList<T> {
    fn drop(&mut self) {
        unsafe { drop(Box::from_raw(self.list_ptr)) }

        let mut node_ptr = self.head_tail.head;

        while let Some(node) = unsafe { node_ptr.as_ref() } {
            let next_ptr = node.next;

            maybe_drop_node(node_ptr);

            node_ptr = next_ptr;
        }
    }
}

pub struct Iter<'a, T> {
    head_tail: HeadTail<T>,
    phantom: PhantomData<&'a T>,
}

impl<'a, T> Iter<'a, T> {
    fn new(head_tail: HeadTail<T>) -> Self {
        Self {
            head_tail,
            phantom: Default::default(),
        }
    }

    fn check_done(&mut self) {
        if self.head_tail.head == self.head_tail.tail {
            self.head_tail.head = std::ptr::null_mut();
            self.head_tail.tail = std::ptr::null_mut();
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        let node = unsafe { self.head_tail.head.as_ref()? };

        self.check_done();

        self.head_tail.head = node.next;

        Some(&node.value)
    }
}

impl<'a, T> DoubleEndedIterator for Iter<'a, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let node = unsafe { self.head_tail.tail.as_ref()? };

        self.check_done();

        self.head_tail.tail = node.prev;

        Some(&node.value)
    }
}

pub struct IntoIter<T> {
    list: LinkedList<T>,
}

impl<T> IntoIter<T> {
    fn new(list: LinkedList<T>) -> Self {
        Self { list }
    }
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.list.pop_head()
    }
}

impl<T> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.list.pop_tail()
    }
}

#[derive(Clone)]
struct HeadTail<T> {
    head: *mut Node<T>,
    tail: *mut Node<T>,
}

struct Node<T> {
    value: T,
    prev: *mut Node<T>,
    next: *mut Node<T>,
    /// Tracks the number of references. There are only two possible references, the one held by
    /// [LinkedList], and the one from [NodeRef].
    ref_count: AtomicUsize,
    removed: AtomicBool,
}

fn maybe_drop_node<T>(node_ptr: *mut Node<T>) {
    let node = unsafe { node_ptr.as_ref().expect("node") };

    let ref_count = node.ref_count.fetch_sub(1, Ordering::Relaxed);

    if ref_count == 1 {
        let node = unsafe { Box::from_raw(node_ptr) };
        drop(node);
    }
}

/// Contains a reference to a value that was added to the [LinkedList].
///
/// Note that the value it points to might be removed from the list while this reference is held.
/// However, there are no race conditions because we always dereference the inner pointer from
/// within the list's functions.
///
/// The actual value will be dropped when both conditions are satisfied:
///
/// 1. The [NodeRef] is dropped and the item is removed, and
/// 2. The item is dropped from the [LinkedList].
pub struct NodeRef<T> {
    /// Used to check whether [NodeRef] is associated to [LinkedList].
    list_ptr: *mut u8,
    /// Reference to actual node in the list.
    node_ptr: *mut Node<T>,
}

unsafe impl<T> Send for NodeRef<T> where T: Send {}
unsafe impl<T> Sync for NodeRef<T> where T: Sync {}

impl<T> Clone for NodeRef<T> {
    fn clone(&self) -> Self {
        unsafe {
            self.node_ptr
                .as_mut()
                .expect("deref")
                .ref_count
                .fetch_add(1, Ordering::Relaxed);
        }

        Self {
            list_ptr: self.list_ptr,
            node_ptr: self.node_ptr,
        }
    }
}

impl<T> NodeRef<T> {
    fn new(list_ptr: *mut u8, node_ptr: *mut Node<T>) -> Self {
        Self { list_ptr, node_ptr }
    }
}

impl<T> Drop for NodeRef<T> {
    fn drop(&mut self) {
        maybe_drop_node(self.node_ptr);
    }
}

impl<T> AsRef<T> for Node<T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<T> Node<T> {
    fn new(value: T) -> Node<T> {
        Node {
            value,
            prev: std::ptr::null_mut(),
            next: std::ptr::null_mut(),
            // One ref for LinkedList, the other for NodeRef.
            ref_count: AtomicUsize::from(2),
            removed: AtomicBool::from(false),
        }
    }

    fn remove(self, head_tail: &mut HeadTail<T>) -> T {
        self.removed.store(true, Ordering::Relaxed);

        let value = self.value;

        assert_eq!(
            head_tail.head.is_null(),
            head_tail.tail.is_null(),
            "head and tail should both be null or non-null"
        );

        if !self.prev.is_null() {
            unsafe { (*self.prev).next = self.next }
        } else {
            // This was the head.
            head_tail.head = self.next
        }

        if !self.next.is_null() {
            unsafe { (*self.next).prev = self.prev }
        } else {
            // This was the tail.
            head_tail.tail = self.prev
        }

        // Last element in the list is both the head and the tail.
        if head_tail.head.is_null() && !head_tail.tail.is_null() {
            head_tail.head = head_tail.tail
        } else if !head_tail.head.is_null() && head_tail.tail.is_null() {
            head_tail.tail = head_tail.head
        }

        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_tail_remove_drop() {
        let mut list = LinkedList::<i32>::new();
        assert!(matches!(list.head(), None));
        assert!(matches!(list.tail(), None));
        let node_ref = list.push_tail(123);
        assert!(matches!(list.head(), Some(&123)));
        assert!(matches!(list.tail(), Some(&123)));
        list.remove(node_ref);
        assert!(matches!(list.head(), None));
        assert!(matches!(list.tail(), None));
        drop(list);
    }

    #[test]
    fn push_head_remove_drop() {
        let mut list = LinkedList::<i32>::new();
        assert!(matches!(list.head(), None));
        assert!(matches!(list.tail(), None));
        let node_ref = list.push_head(123);
        assert!(matches!(list.head(), Some(&123)));
        assert!(matches!(list.tail(), Some(&123)));
        list.remove(node_ref);
        assert!(matches!(list.head(), None));
        assert!(matches!(list.tail(), None));
        drop(list);
    }

    #[test]
    fn collect_iter() {
        let list: LinkedList<_> = (1..=4i32).collect();

        assert_eq!(list.iter().collect::<Vec<_>>(), vec![&1, &2, &3, &4]);
        assert_eq!(list.iter().rev().collect::<Vec<_>>(), vec![&4, &3, &2, &1]);
    }

    #[test]
    fn into_iter_ref() {
        let mut range = 1..=4i32;
        let list: LinkedList<_> = range.clone().collect();
        for got in &list {
            let want = range.next().unwrap();
            assert_eq!(&want, got);
        }
    }

    #[test]
    fn pop_head_tail() {
        let mut list: LinkedList<_> = (1..=4i32).collect();

        assert_eq!(list.pop_head(), Some(1));
        assert_eq!(list.pop_tail(), Some(4));
        assert_eq!(list.pop_head(), Some(2));
        assert_eq!(list.pop_tail(), Some(3));
        assert_eq!(list.pop_head(), None);
        assert_eq!(list.pop_tail(), None);
    }

    #[test]
    fn clone_node_ref() {
        let mut list = LinkedList::new();
        let node_ref1 = list.push_tail(1);
        let node_ref2 = node_ref1.clone();
        let node_ref3 = node_ref2.clone();

        assert_eq!(list.get(&node_ref1), Some(&1));
        assert_eq!(list.get(&node_ref2), Some(&1));
        assert_eq!(list.get(&node_ref3), Some(&1));

        drop(node_ref1);
        assert_eq!(list.get(&node_ref2), Some(&1));
        assert_eq!(list.get(&node_ref3), Some(&1));

        drop(node_ref2);
        assert_eq!(list.get(&node_ref3), Some(&1));

        drop(node_ref3);
        assert_eq!(list.pop_head(), Some(1));
        assert_eq!(list.pop_head(), None);
    }

    #[test]
    fn add_remove_multiple_drop() {
        let mut list = LinkedList::<i32>::new();
        assert!(matches!(list.head(), None));
        assert!(matches!(list.tail(), None));
        let node_ref1 = list.push_tail(1);
        let node_ref2 = list.push_tail(2);
        let node_ref3 = list.push_tail(3);
        let node_ref4 = list.push_tail(4);

        assert!(matches!(list.head(), Some(&1)));
        assert!(matches!(list.tail(), Some(&4)));
        assert_eq!(list.iter().collect::<Vec<_>>(), vec![&1, &2, &3, &4]);

        list.remove(node_ref1);
        assert!(matches!(list.head(), Some(&2)));
        assert!(matches!(list.tail(), Some(&4)));
        assert_eq!(list.iter().collect::<Vec<_>>(), vec![&2, &3, &4]);

        list.remove(node_ref3);
        assert!(matches!(list.head(), Some(&2)));
        assert!(matches!(list.tail(), Some(&4)));
        assert_eq!(list.iter().collect::<Vec<_>>(), vec![&2, &4]);

        list.remove(node_ref4);
        assert!(matches!(list.head(), Some(&2)));
        assert!(matches!(list.tail(), Some(&2)));
        assert_eq!(list.iter().collect::<Vec<_>>(), vec![&2]);

        list.remove(node_ref2);
        assert!(matches!(list.head(), None));
        assert!(matches!(list.tail(), None));
        assert_eq!(list.iter().collect::<Vec<_>>(), Vec::<&i32>::new());
    }
}
