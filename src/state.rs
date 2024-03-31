use std::ops::DerefMut;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::linked_list::{LinkedList, NodeRef};
use crate::queue::{Queue, Spot};
use crate::{Receiver, ReserveError, Sender, TryReserveError};

pub struct State<T> {
    inner: Arc<Mutex<InnerState<T>>>,
}

impl<T> State<T> {}

impl<T> Clone for State<T>
where
    T: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> State<T>
where
    T: Send + Sync + 'static,
{
    pub fn new(cap: usize) -> Self {
        State {
            inner: Arc::new(Mutex::new(InnerState::new(cap))),
        }
    }

    pub fn inner_mut(&self) -> impl DerefMut<Target = InnerState<T>> + '_ {
        self.inner.lock().unwrap()
    }

    // fn next_waker_id(&self) -> wakerid {
    //     self.inner_mut().next_waker_id()
    // }

    pub fn new_sender(&self) -> Sender<T> {
        self.inner_mut().senders_count += 1;
        Sender::new(self.clone())
    }

    pub fn new_receiver(&self) -> Receiver<T> {
        self.inner_mut().receivers_count += 1;
        Receiver::new(self.clone())
    }

    // fn wake_all(wakers: option<impl iterator<item = waker>>) {
    //     if let some(wakers) = wakers {
    //         for waker in wakers {
    //             waker.wake()
    //         }
    //     }
    // }

    pub fn close_all_receivers(&self) {
        let (send_futures, recv_futures) = {
            let mut inner = self.inner_mut();
            // inner.disconnected = true;

            (
                inner.mark_disconnected_and_take_send_futures(),
                inner.mark_disconnected_and_take_recv_futures(),
            )
        };

        for send_future in send_futures {
            send_future.wake();
        }

        for recv_future in recv_futures {
            recv_future.wake()
        }
    }

    pub fn drop_sender(&self) {
        let recv_futures = {
            let mut inner = self.inner_mut();
            inner.senders_count -= 1;

            (inner.senders_count == 0).then(|| inner.mark_disconnected_and_take_recv_futures())
        };

        if let Some(recv_futures) = recv_futures {
            for recv_future in recv_futures {
                recv_future.wake();
            }
        }
    }

    pub fn drop_receiver(&self) {
        let send_futures = {
            let mut inner = self.inner_mut();
            inner.receivers_count -= 1;

            (inner.receivers_count == 0).then(|| inner.mark_disconnected_and_take_send_futures())
        };

        if let Some(send_futures) = send_futures {
            for send_future in send_futures {
                send_future.wake();
            }
        }
    }

    pub fn try_reserve(&self, count: usize) -> Result<NodeRef<Spot<T>>, TryReserveError> {
        let mut inner = self.inner_mut();
        inner.try_reserve(count)
    }

    pub fn reserve(
        &self,
        count: usize,
        cx: &mut Context<'_>,
        send_future: &mut Option<NodeRef<SendWaker>>,
    ) -> Poll<Result<NodeRef<Spot<T>>, ReserveError>> {
        let mut inner = self.inner_mut();

        match inner.try_reserve(count) {
            Ok(reservation) => {
                if let Some(send_future) = send_future.take() {
                    inner.send_futures.remove(send_future);
                }

                Poll::Ready(Ok(reservation))
            }
            Err(TryReserveError::Full) => {
                if send_future.is_none() {
                    *send_future = Some(inner.send_futures.push_tail(SendWaker {
                        count,
                        waker: cx.waker().clone(),
                    }));
                }

                Poll::Pending
            }
            Err(TryReserveError::Disconnected) => Poll::Ready(Err(ReserveError)),
        }
    }

    pub fn send_with_permit(&self, reservation: NodeRef<Spot<T>>, value: T) {
        let waker = {
            let mut inner = self.inner_mut();
            inner.queue.send(reservation, value);
            inner.recv_futures.pop_head()
            // TODO figure out if this is needed instead?
            // inner.recv_futures.head().map(|waker| waker.clone())
        };

        if let Some(waker) = waker {
            waker.wake()
        }
    }

    pub fn drop_permit(&self, reservation: NodeRef<Spot<T>>, count: usize) {
        let waker = {
            let mut inner = self.inner_mut();

            let released = inner.queue.release(reservation, count);

            // When the permit was not used for sending, it means a spot was freed, so we can
            // notify the next sender that it can proceed.
            released
                .then(|| {
                    inner
                        .send_futures
                        .head()
                        .filter(|send_waker| inner.queue.has_room_for(send_waker.count))
                        .map(|send_waker| send_waker.clone())
                })
                .flatten()
        };

        if let Some(waker) = waker {
            waker.wake();
        }
    }

    // fn drop_send_future(&self, fut: Option<NodeRef<SendWaker>>) {
    //     let waker = {
    //         let mut inner = self.inner_mut();

    //         if let Some(fut) = fut {
    //             inner.waiting_send_futures.remove(fut);
    //             inner.reserved_count -= fut.count
    //         }

    //         // If there's room for the next sender in the queue, wake it.
    //         match inner.waiting_send_futures.head() {
    //             Some(next) => inner.has_room_for(next.count).then(|| next.waker.clone()),
    //             None => None,
    //         }
    //     };

    //     if let Some(waker) = waker {
    //         waker.wake();
    //     }
    // }

    // fn drop_recv_future(&self, id: &WakerId, has_received: bool) {
    //     let waker = {
    //         let mut inner = self.inner_mut();
    //         let was_awoken = inner.waiting_recv_futures.remove(id).is_none();

    //         // Wake another receiver in case this one has been awoken, but it was dropped before it
    //         // managed to receive anything.
    //         if was_awoken {
    //             if has_received {
    //                 // If we have received, it means a spot was freed in the internal buffer, so wake
    //                 // one sender.
    //                 inner.take_one_send_future_waker()
    //             } else {
    //                 // If we have not received, it means another RecvFuture might take over.
    //                 inner.take_one_recv_future_waker()
    //             }
    //         } else {
    //             None
    //         }
    //     };

    //     if let Some(waker) = waker {
    //         waker.wake();
    //     }
    // }
}

#[derive(Clone)]
pub struct SendWaker {
    count: usize,
    waker: Waker,
}

impl SendWaker {
    pub fn new(count: usize, waker: Waker) -> Self {
        Self { count, waker }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn wake(self) {
        self.waker.wake()
    }

    pub fn wake_by_ref(&self) {
        self.waker.wake_by_ref();
    }
}

// TODO we need acustom impl of a LinkedList to be able to add/remove wakers, instead of using
// WakerId.
//
// Moreover, currently reservations are just a counter, they don't actually take the
// spot in the buffer, so in the case of a single consumer the result will be
// re-ordered. We could keep an enum like this:
//
// ```
// enum Spot<T> {
//   Value(T),
//   Reserved,
// }
// ```
//
// in the buffer so that we know we need to wait for the reservations.
struct InnerState<T> {
    queue: Queue<T>,
    /// Total of created receivers.
    receivers_count: usize,
    /// Total of created senders.
    senders_count: usize,
    /// Senders waiting to send.
    send_futures: LinkedList<SendWaker>,
    /// Receivers waiting to receive.
    recv_futures: LinkedList<Waker>,
    /// False by default, true when all senders or all receivers are dropped.
    disconnected: bool,
}

impl<T> InnerState<T>
where
    T: Send + Sync + 'static,
{
    pub fn new(cap: usize) -> Self {
        Self {
            queue: Queue::new(cap),
            receivers_count: 0,
            senders_count: 0,
            send_futures: LinkedList::new(),
            recv_futures: LinkedList::new(),
            disconnected: false,
        }
    }

    pub fn try_reserve(&mut self, count: usize) -> Result<NodeRef<Spot<T>>, TryReserveError> {
        if self.disconnected {
            Err(TryReserveError::Disconnected)
        } else {
            self.queue.try_reserve(count).ok_or(TryReserveError::Full)
        }
    }

    // #[must_use]
    // fn send_and_get_receiver_waker(
    //     &mut self,
    //     value: T,
    //     waker: NodeRef<SendWaker>,
    // ) -> Option<&Waker> {
    //     self.buffer.push_back(value);
    //     self.send_futures.remove(waker);
    //     self.recv_futures.head()
    // }

    fn mark_disconnected_and_take_send_futures(&mut self) -> LinkedList<SendWaker> {
        self.disconnected = true;
        std::mem::take(&mut self.send_futures)
    }

    fn mark_disconnected_and_take_recv_futures(&mut self) -> LinkedList<Waker> {
        self.disconnected = true;
        std::mem::take(&mut self.recv_futures)
    }

    // #[must_use]
    // fn take_one_send_future_waker(&mut self) -> Option<Waker> {
    //     if self.has_room_for(1) {
    //         Self::take_one_waker(&mut self.send_futures)
    //     } else {
    //         None
    //     }
    // }

    // #[must_use]
    // fn take_one_recv_future_waker(&mut self) -> Option<Waker> {
    //     if !self.buffer.is_empty() {
    //         Self::take_one_waker(&mut self.recv_futures)
    //     } else {
    //         None
    //     }
    // }

    // #[must_use]
    // fn take_one_waker(map: &mut BTreeMap<WakerId, Waker>) -> Option<Waker> {
    //     map.pop_first().map(|(_, waker)| waker)
    // }

    // fn take_all_wakers(map: &mut BTreeMap<WakerId, Waker>) -> BTreeMap<WakerId, Waker> {
    //     std::mem::take(map)
    // }
}
