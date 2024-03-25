use std::marker::PhantomData;

/// Implements a double-linked list with O(1) complexity for:
/// - `push` operation, and
/// - `remove` of _any_ node
///
/// with the following caveat:
///
/// For every call to `push` `remove` MUST be called before the resulting `NodeRef` is dropped,
/// or else dropping `NodeRef` will panic.
pub struct LinkedList<T> {
    head_tail: HeadTail<T>,
    len: usize,
}

impl<T> LinkedList<T> {
    pub fn new() -> Self {
        Self {
            head_tail: HeadTail {
                head: std::ptr::null_mut(),
                tail: std::ptr::null_mut(),
            },
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Pushes an element to the tail of the double linked list with O(1) complexity. After calling
    /// this method users MUST remove the element from the list before the resulting `NodeRef` is
    /// dropped.
    #[must_use]
    pub fn push(&mut self, value: T) -> NodeRef<T> {
        self.len += 1;

        let mut node = Node::new(value);

        assert_eq!(
            self.head_tail.head.is_null(),
            self.head_tail.tail.is_null(),
            "head and tail should both be null or non-null"
        );

        let tail = unsafe {
            self.head_tail.tail.as_mut()
        };

        match tail {
            None => {
                let node_ptr = Box::into_raw(Box::new(node));
                self.head_tail.head = node_ptr;
                self.head_tail.tail = node_ptr;
                NodeRef(node_ptr)
            },
            Some(tail) => {
                node.prev = self.head_tail.tail;
                let node_ptr = Box::into_raw(Box::new(node));
                tail.next = node_ptr;
                self.head_tail.tail = node_ptr;
                NodeRef(node_ptr)
            },
        }
    }

    /// Returns a reference to the first element, if any.
    pub fn head(&self) -> Option<&T> {
        unsafe { self.head_tail.head.as_ref().map(|node| &node.value) }
    }

    /// Returns a reference to the first element, if any.
    pub fn tail(&self) -> Option<&T> {
        unsafe { self.head_tail.tail.as_ref().map(|node| &node.value) }
    }

    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            next: self.head_tail.head,
            phantom: Default::default(),
        }
    }

    /// Removes the element with O(1) complexity.
    pub fn remove(&mut self, mut node_ref: NodeRef<T>) -> T {
        let node_ptr = node_ref.0;

        let node = unsafe { Box::from_raw(node_ptr) };

        let value = node.remove(&mut self.head_tail);

        // Don't panic in drop.
        node_ref.0 = std::ptr::null_mut();

        println!("setting node_ref.0 to null");

        drop(node_ref);

        self.len -= 1;

        value
    }
}

struct Iter<'a, T> {
    next: *mut Node<T>,
    phantom: PhantomData<&'a T>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        let node = unsafe { self.next.as_ref()? };

        self.next = node.next;

        Some(&node.value)
    }
}

struct HeadTail<T> {
    head: *mut Node<T>,
    tail: *mut Node<T>,
}

struct Node<T> {
    value: T,
    prev: *mut Node<T>,
    next: *mut Node<T>,
}

pub struct NodeRef<T>(*mut Node<T>);

impl<T> AsRef<T> for NodeRef<T> {
    fn as_ref(&self) -> &T {
        unsafe { &(*self.0).value }
    }
}

impl<T> NodeRef<T> {
    fn as_node(&self) -> &Node<T> {
        unsafe { &(*self.0) }
    }
}

impl<T> AsRef<T> for Node<T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<T> Drop for NodeRef<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let node = unsafe { &*self.0 };
            panic!("NodeRef dropped without beng removed from the LinkedList")
        }
    }
}

impl<T> Node<T> {
    fn new(value: T) -> Node<T> {
        Node {
            value,
            prev: std::ptr::null_mut(),
            next: std::ptr::null_mut(),
        }
    }

    fn remove(self, head_tail: &mut HeadTail<T>) -> T {
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
    fn add_remove_drop() {
        let mut list = LinkedList::<i32>::new();
        assert!(matches!(list.head(), None));
        assert!(matches!(list.tail(), None));
        let node_ref = list.push(123);
        assert!(matches!(list.head(), Some(&123)));
        assert!(matches!(list.tail(), Some(&123)));
        list.remove(node_ref);
        assert!(matches!(list.head(), None));
        assert!(matches!(list.tail(), None));
        drop(list);
    }

    #[test]
    fn add_remove_multiple_drop() {
        let mut list = LinkedList::<i32>::new();
        assert!(matches!(list.head(), None));
        assert!(matches!(list.tail(), None));
        let node_ref1 = list.push(1);
        let node_ref2 = list.push(2);
        let node_ref3 = list.push(3);
        let node_ref4 = list.push(4);

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
