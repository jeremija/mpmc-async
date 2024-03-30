use std::fmt::Debug;

use crate::linked_list::{LinkedList, NodeRef};

/// A wrapper around [LinkedList] that processes reservations, writes and reads.
///
/// The implemented methods panic when used incorrectly. This is because incorrect usage is a bug
/// in the codebase and any sort of error handling would result in incorrect behavior.
pub struct Queue<T> {
    list: LinkedList<Spot<T>>,
    len: usize,
    cap: usize,
}

impl<T> Queue<T> {
    /// Creates a new instance with the provided capacity.
    pub fn new(cap: usize) -> Self {
        Self {
            list: LinkedList::new(),
            len: 0,
            cap,
        }
    }

    /// Returns the number of queued items, including values and reserved spots.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns the maximum number of items in the Queue. This value never changes.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Attempts to reserve a number of spots in the queue if there is room.
    pub fn try_reserve(&mut self, count: usize) -> Option<NodeRef<Spot<T>>> {
        if self.len + count > self.cap {
            return None;
        }

        let reservation = self.list.push_tail(Spot::Reserved(count));

        Some(reservation)
    }

    /// Reads the next ready value, if any.
    pub fn read_next_value(&mut self) -> Option<T> {
        let Spot::Value(_) = self.list.head()? else {
            return None;
        };

        let spot = self.list.pop_head().expect("no value to pop: just checked");

        match spot {
            Spot::Value(value) => Some(value),
            Spot::Reserved(_) => unreachable!(),
        }
    }

    /// Replaces a single reserved spots with a value.
    pub fn write(&mut self, reservation: &NodeRef<Spot<T>>, value: T) {
        let spot = self
            .list
            .get_mut(reservation)
            .expect("reservation not found");

        let count = match spot {
            Spot::Value(_) => panic!("illegal: called set_value on value"),
            Spot::Reserved(count) => count,
        };

        if *count == 0 {
            panic!("invalid state: found reservation with zero spots left");
        }

        *count -= 1;

        let value = Spot::Value(value);

        if *count == 0 {
            *spot = value;
        } else {
            self.list
                .push_before(reservation, value)
                .expect("push_before failed");
        }
    }

    pub fn release(&mut self, reservation: NodeRef<Spot<T>>, release: Release) {
        match release {
            Release::All => {
                let spot = self
                    .list
                    .remove(reservation)
                    .expect("reservation not found");

                let count = match spot {
                    Spot::Reserved(count) => count,
                    Spot::Value(_) => panic!("illegal: called release on value"),
                };

                self.len -= count;
            }
            Release::Count(count_to_remove) => {
                let spot = self
                    .list
                    .get_mut(&reservation)
                    .expect("reservation not found");

                let count = match spot {
                    Spot::Reserved(count) => count,
                    Spot::Value(_) => panic!("illegal: called release on value"),
                };

                if count_to_remove > *count {
                    panic!("count_to_remove {count_to_remove} > count {count}");
                }

                if *count == count_to_remove {
                    self.list
                        .remove(reservation)
                        .expect("reservation not found");
                } else {
                    *count -= count_to_remove;
                }
            }
        };
    }
}

/// Describes how many reserved spots to release.
pub enum Release {
    /// Uesd to release all reserved spots for the reference (e.g. in case of dropping an
    /// iterator).
    All,
    /// Used to release a single reserved spot for the reference (e.g. for a single permit).
    Count(usize),
}

/// A spot in the queue.
pub enum Spot<T> {
    /// An actual value that is meant to be read.
    Value(T),
    /// A reserved spot which will block the reads once it is the until a value is written or the
    /// spot released.
    Reserved(usize),
}

impl<T> Debug for Spot<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Value(_) => f.write_str("Spot::Value"),
            Self::Reserved(count) => write!(f, "Spot::Count({count})"),
        }
    }
}
