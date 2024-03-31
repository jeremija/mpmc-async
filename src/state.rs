use std::ops::DerefMut;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::linked_list::{LinkedList, NodeRef};
use crate::queue::{Queue, Recv, Spot};
use crate::{Receiver, RecvError, ReserveError, Sender, TryRecvError, TryReserveError};

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

    fn inner_mut(&self) -> impl DerefMut<Target = InnerState<T>> + '_ {
        self.inner.lock().unwrap()
    }

    pub fn new_sender(&self) -> Sender<T> {
        self.inner_mut().senders_count += 1;
        Sender::new(self.clone())
    }

    pub fn new_receiver(&self) -> Receiver<T> {
        self.inner_mut().receivers_count += 1;
        Receiver::new(self.clone())
    }

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
        waker_ref: &mut Option<NodeRef<SendWaker>>,
    ) -> Poll<Result<NodeRef<Spot<T>>, ReserveError>> {
        let mut inner = self.inner_mut();

        match inner.try_reserve(count) {
            Ok(reservation) => {
                if let Some(send_future) = waker_ref.take() {
                    inner.send_futures.remove(send_future);
                }

                Poll::Ready(Ok(reservation))
            }
            Err(TryReserveError::Full) => {
                let send_future = SendWaker::new(count, cx.waker().clone());

                match waker_ref {
                    None => {
                        *waker_ref = Some(inner.send_futures.push_tail(send_future));
                    }
                    Some(waker_ref) => {
                        // Satisfying the following requirement from std::future::Future::poll
                        // docs:
                        //
                        //     Note that on multiple calls to poll, only the Waker from the Context
                        //     passed to the most recent call should be scheduled to receive a
                        //     wakeup.
                        let send_future_mut =
                            inner.send_futures.get_mut(waker_ref).expect("send_future");
                        *send_future_mut = send_future;
                    }
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
            inner.recv_futures.head().cloned()
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
                        .map(|send_waker| send_waker.waker.clone())
                })
                .flatten()
        };

        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let mut inner = self.inner_mut();
        inner.try_recv()
    }

    pub fn recv(
        &self,
        cx: &mut Context<'_>,
        waker_ref: &mut Option<NodeRef<Waker>>,
    ) -> Poll<Result<T, RecvError>> {
        self.recv_with_callback(cx, waker_ref, |value, _inner| value)
    }

    fn recv_with_callback<F, R>(
        &self,
        cx: &mut Context<'_>,
        waker_ref: &mut Option<NodeRef<Waker>>,
        callback: F,
    ) -> Poll<Result<R, RecvError>>
    where
        F: FnOnce(T, &mut InnerState<T>) -> R,
    {
        let mut inner = self.inner_mut();

        match inner.try_recv() {
            Ok(value) => {
                if let Some(node_ref) = waker_ref.take() {
                    inner.recv_futures.remove(node_ref);
                }

                let ret = callback(value, &mut inner);

                Poll::Ready(Ok(ret))
            }
            Err(TryRecvError::Disconnected) => Poll::Ready(Err(RecvError)),
            Err(TryRecvError::Empty) => {
                let waker = cx.waker().clone();
                match waker_ref {
                    None => {
                        *waker_ref = Some(inner.recv_futures.push_tail(waker));
                    }
                    Some(waker_ref) => {
                        // Satisfying the following requirement from std::future::Future::poll
                        // docs:
                        //
                        //     Note that on multiple calls to poll, only the Waker from the Context
                        //     passed to the most recent call should be scheduled to receive a
                        //     wakeup.
                        let waker_ref_mut =
                            inner.recv_futures.get_mut(waker_ref).expect("recv_future");
                        *waker_ref_mut = waker;
                    }
                }

                Poll::Pending
            }
        }
    }

    pub fn try_recv_many(&self, vec: &mut Vec<T>, count: usize) -> Result<usize, TryRecvError> {
        let mut inner = self.inner_mut();
        inner.try_recv_many(vec, count)
    }

    pub fn recv_many(
        &self,
        cx: &mut Context<'_>,
        waker_ref: &mut Option<NodeRef<Waker>>,
        vec: &mut Vec<T>,
        count: usize,
    ) -> Poll<Result<usize, RecvError>> {
        self.recv_with_callback(cx, waker_ref, |value, inner| {
            vec.push(value);
            inner.fill_rest(vec, count)
        })
    }

    pub fn drop_recv_future(&self, waker_ref: &mut Option<NodeRef<Waker>>) {
        let waker = {
            let mut inner = self.inner_mut();

            if let Some(node_ref) = waker_ref.take() {
                inner.recv_futures.remove(node_ref);
            }

            let has_received = waker_ref.is_none();

            if has_received {
                // If we have received, it means a spot was freed in the internal buffer, so wake
                // one sender.
                inner.next_send_future_waker()
            } else {
                // If we have not received, it means another RecvFuture might take over.
                inner.next_recv_future_waker()
            }
        };

        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

pub struct SendWaker {
    count: usize,
    waker: Waker,
}

impl SendWaker {
    pub fn new(count: usize, waker: Waker) -> Self {
        Self { count, waker }
    }

    pub fn wake(self) {
        self.waker.wake()
    }
}

struct InnerState<T> {
    /// Contains ordered values and pending entries.
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

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        match self.queue.try_recv() {
            Recv::Value(value) => Ok(value),
            Recv::Pending => Err(TryRecvError::Empty),
            Recv::Empty => {
                if self.disconnected {
                    Err(TryRecvError::Disconnected)
                } else {
                    Err(TryRecvError::Empty)
                }
            }
        }
    }

    pub fn try_recv_many(&mut self, vec: &mut Vec<T>, count: usize) -> Result<usize, TryRecvError> {
        let value = self.try_recv()?;
        vec.push(value);
        Ok(self.fill_rest(vec, count))
    }

    pub fn fill_rest(&mut self, vec: &mut Vec<T>, count: usize) -> usize {
        let mut total = 1;

        for _ in 1..count {
            match self.queue.try_recv() {
                Recv::Value(value) => {
                    vec.push(value);
                    total += 1;
                }
                Recv::Pending | Recv::Empty => break,
            }
        }

        total
    }

    fn mark_disconnected_and_take_send_futures(&mut self) -> LinkedList<SendWaker> {
        self.disconnected = true;
        std::mem::take(&mut self.send_futures)
    }

    fn mark_disconnected_and_take_recv_futures(&mut self) -> LinkedList<Waker> {
        self.disconnected = true;
        std::mem::take(&mut self.recv_futures)
    }

    #[must_use]
    fn next_send_future_waker(&self) -> Option<Waker> {
        let send_future = self.send_futures.head()?;

        if self.queue.has_room_for(send_future.count) {
            // NOTE: Not calling pop_head() because calling wake() does not guarantee that the
            // future will be chosen (e.g. in a tokio::select!)
            Some(send_future.waker.clone())
        } else {
            None
        }
    }

    #[must_use]
    fn next_recv_future_waker(&mut self) -> Option<Waker> {
        if !self.queue.can_recv() {
            // NOTE: Not calling pop_head() because calling wake() does not guarantee that the future will be
            // chosen (e.g. in a tokio::select!)
            self.recv_futures.head().cloned()
        } else {
            None
        }
    }
}
