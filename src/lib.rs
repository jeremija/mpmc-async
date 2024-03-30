///! A multi-producer, multi-consumer async channel with reservations.
///!
///! Example usage:
///!
///! ```rust
///! tokio_test::block_on(async {
///!     let (tx1, rx1) = mpmc_async::channel(2);
///!
///!     let task = tokio::spawn(async move {
///!       let rx2 = rx1.clone();
///!       assert_eq!(rx1.recv().await.unwrap(), 1);
///!       assert_eq!(rx2.recv().await.unwrap(), 2);
///!     });
///!
///!     let tx2 = tx1.clone();
///!     let permit = tx1.reserve().await.unwrap();
///!     tx2.send(1).await.unwrap();
///!     permit.send(2);
///!
///!     task.await.unwrap();
///! });
///! ```
///!
///! A more complex example with multiple sender and receiver tasks:
///!
///! ```rust
///! use std::collections::BTreeSet;
///! use std::ops::DerefMut;
///! use std::sync::{Arc, Mutex};
///!
///! tokio_test::block_on(async {
///!     let (tx, rx) = mpmc_async::channel(1);
///!
///!     let num_workers = 10;
///!     let count = 10;
///!     let mut tasks = Vec::with_capacity(num_workers);
///!
///!     for i in 0..num_workers {
///!         let mut tx = tx.clone();
///!         let task = tokio::spawn(async move {
///!             for j in 0..count {
///!                 let val = i * count + j;
///!                 tx.reserve().await.expect("no error").send(val);
///!             }
///!         });
///!         tasks.push(task);
///!     }
///!
///!     let total = count * num_workers;
///!     let values = Arc::new(Mutex::new(BTreeSet::new()));
///!
///!     for _ in 0..num_workers {
///!         let values = values.clone();
///!         let rx = rx.clone();
///!         let task = tokio::spawn(async move {
///!             for _ in 0..count {
///!                 let val = rx.recv().await.expect("Failed to recv");
///!                 values.lock().unwrap().insert(val);
///!             }
///!         });
///!         tasks.push(task);
///!     }
///!
///!     for task in tasks {
///!         task.await.expect("failed to join task");
///!     }
///!
///!     let exp = (0..total).collect::<Vec<_>>();
///!     let got = std::mem::take(values.lock().unwrap().deref_mut())
///!         .into_iter()
///!         .collect::<Vec<_>>();
///!     assert_eq!(exp, got);
///! });
///! ```
pub mod linked_list;
pub mod queue;

// use std::collections::{BTreeMap, VecDeque};
//use std::fmt::Display;
//use std::ops::DerefMut;
//use std::pin::Pin;
//use std::sync::{Arc, Mutex};
//use std::task::{Context, Poll, Waker};

//use std::future::Future;

//use self::linked_list::{LinkedList, NodeRef};

///// Occurs when all receivers have been dropped.
//#[derive(PartialEq, Eq, Clone, Copy, Debug)]
//pub struct SendError<T>(pub T);

//impl<T> Display for SendError<T> {
//    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//        f.write_str("Send failed: disconnected")
//    }
//}

///// TrySendError occurs when the channel is empty or all receivers have been dropped.
//#[derive(PartialEq, Eq, Clone, Copy, Debug)]
//pub enum TrySendError<T> {
//    /// Channel full
//    Full(T),
//    /// Disconnected
//    Disconnected(T),
//}

//impl<T> Display for TrySendError<T> {
//    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//        match self {
//            TrySendError::Full(_) => f.write_str("Try send failed: full"),
//            TrySendError::Disconnected(_) => f.write_str("Try send failed: disconnected"),
//        }
//    }
//}

///// Occurs when all receivers have been dropped.
//#[derive(PartialEq, Eq, Clone, Copy, Debug)]
//pub struct ReserveError;

//impl Display for ReserveError {
//    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//        f.write_str("Reserve failed: disconnected")
//    }
//}

///// Occurs when the channel is full, or all receivers have been dropped.
//#[derive(PartialEq, Eq, Clone, Copy, Debug)]
//pub enum TryReserveError {
//    /// Channel full
//    Full,
//    /// Disconnected
//    Disconnected,
//}

//impl TryReserveError {
//    fn into_try_send_error<T>(self, value: T) -> TrySendError<T> {
//        match self {
//            TryReserveError::Full => TrySendError::Full(value),
//            TryReserveError::Disconnected => TrySendError::Disconnected(value),
//        }
//    }
//}

//impl Display for TryReserveError {
//    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//        match self {
//            TryReserveError::Full => f.write_str("Try send failed: full"),
//            TryReserveError::Disconnected => f.write_str("Try send failed: disconnected"),
//        }
//    }
//}

///// Occurs when all senders have been dropped.
//#[derive(PartialEq, Eq, Clone, Copy, Debug)]
//pub struct RecvError;

//impl Display for RecvError {
//    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//        f.write_str("Recv failed: disconnected")
//    }
//}

///// Occurs when channel is empty or all senders have been dropped.
//#[derive(PartialEq, Eq, Clone, Copy, Debug)]
//pub enum TryRecvError {
//    /// Channel is empty
//    Empty,
//    /// Disconnected
//    Disconnected,
//}

//impl Display for TryRecvError {
//    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//        match self {
//            TryRecvError::Empty => f.write_str("Try recv failed: empty"),
//            TryRecvError::Disconnected => f.write_str("Try recv failed: disconnected"),
//        }
//    }
//}

///// Creates a new bounded channel. When `cap` is 0 it will be increased to 1.
//pub fn channel<T>(mut cap: usize) -> (Sender<T>, Receiver<T>)
//where
//    T: Send + Sync + 'static,
//{
//    if cap == 0 {
//        cap = 1
//    }

//    let state = State {
//        inner: Arc::new(Mutex::new(InnerState {
//            buffer: VecDeque::with_capacity(cap),
//            reserved_count: 0,
//            receivers_count: 0,
//            senders_count: 0,
//            waiting_send_futures: LinkedList::new(),
//            waiting_recv_futures: LinkedList::new(),
//            disconnected: false,
//        })),
//    };

//    (state.new_sender(), state.new_receiver())
//}

///// Receives messages sent by [Sender].
//pub struct Receiver<T>
//where
//    T: Send + Sync + 'static,
//{
//    state: State<T>,
//}

///// Cloning creates new a instance with the shared state  and increases the internal reference
///// counter. It is guaranteed that a single message will be distributed to exacly one receiver
///// future awaited after calling `recv()`.
//impl<T> Clone for Receiver<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn clone(&self) -> Self {
//        self.state.new_receiver()
//    }
//}

//impl<T> Receiver<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn new(state: State<T>) -> Self {
//        Self { state }
//    }

//    /// Disconnects all receivers from senders. The receivers will still receive all buffered
//    /// or reserved messages before returning an error, allowing a graceful teardown.
//    pub fn close_all(&self) {
//        self.state.close_all_receivers();
//    }

//    /// Waits until there's a message to be read and returns. Returns an error when there are no
//    /// more messages in the queue and all [Sender]s have been dropped.
//    pub async fn recv(&self) -> Result<T, RecvError> {
//        let recv = RecvFuture::new(self);

//        recv.await
//    }

//    /// Checks if there's a message to be read and returns immediately. Returns an error when the
//    /// channel is disconnected or empty.
//    pub fn try_recv(&self) -> Result<T, TryRecvError> {
//        let mut state = self.state.inner_mut();

//        if let Some(value) = state.buffer.pop_front() {
//            Ok(value)
//        } else if state.disconnected && state.reserved_count == 0 {
//            Err(TryRecvError::Disconnected)
//        } else {
//            Err(TryRecvError::Empty)
//        }
//    }

//    pub async fn recv_many(&self, vec: &mut Vec<T>, count: usize) -> Result<usize, RecvError> {
//        let recv = RecvManyFuture::new(self.state.next_waker_id(), self, vec, count);
//        recv.await
//    }

//    pub fn try_recv_many(&self, vec: &mut Vec<T>, count: usize) -> Result<usize, TryRecvError> {
//        let mut state = self.state.inner_mut();

//        if let Some(value) = state.buffer.pop_front() {
//            let mut num_received = 1;
//            vec.push(value);

//            for _ in 1..count {
//                match state.buffer.pop_front() {
//                    Some(value) => {
//                        vec.push(value);
//                        num_received += 1;
//                    }
//                    None => break,
//                }
//            }

//            Ok(num_received)
//        } else if state.disconnected && state.reserved_count == 0 {
//            Err(TryRecvError::Disconnected)
//        } else {
//            Err(TryRecvError::Empty)
//        }
//    }
//}

///// The last reciever that's dropped will mark the channel as disconnected.
//impl<T> Drop for Receiver<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn drop(&mut self) {
//        self.state.drop_receiver();
//    }
//}

///// Producers messages to be read by [Receiver]s.
//#[derive(Debug)]
//pub struct Sender<T>
//where
//    T: Send + Sync + 'static,
//{
//    state: State<T>,
//}

///// Cloning creates new a instance with the shared state  and increases the internal reference
///// counter.
//impl<T> Clone for Sender<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn clone(&self) -> Self {
//        self.state.new_sender()
//    }
//}

//impl<T> Sender<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn new(state: State<T>) -> Sender<T> {
//        Self { state }
//    }

//    /// Waits until the value is sent or returns an when all receivers have been dropped.
//    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
//        match self.reserve().await {
//            Ok(permit) => {
//                permit.send(value);
//                Ok(())
//            }
//            Err(err) => Err(err.into_try_send_error(value)),
//        }
//    }

//    /// Sends without blocking or returns an when all receivers have been dropped.
//    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
//        let mut state = self.state.inner_mut();

//        if state.disconnected {
//            return Err(TrySendError::Disconnected(value));
//        }

//        if state.has_room_for(1) {
//            if let Some(waker) = state.send_and_get_waker(value) {
//                drop(state);
//                waker.wake();
//            }
//            Ok(())
//        } else {
//            Err(TrySendError::Full(value))
//        }
//    }

//    /// Waits until a permit is reserved or returns an when all receivers have been dropped.
//    pub async fn reserve(&self) -> Result<Permit<'_, T>, ReserveError> {
//        ReserveFuture::new(self.state.next_waker_id(), &self.state, 1).await?;
//        Ok(Permit::new(self))
//    }

//    /// Reserves a permit or returns an when all receivers have been dropped.
//    pub fn try_reserve(&self) -> Result<Permit<'_, T>, TryReserveError> {
//        let mut state = self.state.inner_mut();

//        if state.disconnected {
//            return Err(TryReserveError::Disconnected);
//        }

//        if state.has_room_for(1) {
//            state.reserved_count += 1;
//            Ok(Permit::new(self))
//        } else {
//            Err(TryReserveError::Full)
//        }
//    }

//    /// Waits until multiple Permits in the queue are reserved.
//    pub async fn reserve_many(&self, count: usize) -> Result<PermitIterator<'_, T>, ReserveError> {
//        ReserveFuture::new(self.state.next_waker_id(), &self.state, count).await?;
//        Ok(PermitIterator::new(self, count))
//    }

//    /// Reserves multiple Permits in the queue, or errors out when there's no room.
//    pub fn try_reserve_many(&self, count: usize) -> Result<PermitIterator<'_, T>, TryReserveError> {
//        let mut state = self.state.inner_mut();

//        if state.disconnected {
//            return Err(TryReserveError::Disconnected);
//        }

//        if state.has_room_for(count) {
//            state.reserved_count += count;
//            Ok(PermitIterator::new(self, count))
//        } else {
//            Err(TryReserveError::Full)
//        }
//    }

//    /// Like [Sender::reserve], but takes ownership of `Sender` until sending is done.
//    pub async fn reserve_owned(self) -> Result<OwnedPermit<T>, ReserveError> {
//        ReserveFuture::new(self.state.next_waker_id(), &self.state, 1).await?;
//        Ok(OwnedPermit::new(self))
//    }

//    /// Reserves a permit or returns an when all receivers have been dropped.
//    pub fn try_reserve_owned(self) -> Result<OwnedPermit<T>, TrySendError<Self>> {
//        let mut state = self.state.inner_mut();

//        if state.disconnected {
//            drop(state);
//            return Err(TrySendError::Disconnected(self));
//        }

//        if state.has_room_for(1) {
//            state.reserved_count += 1;
//            drop(state);
//            Ok(OwnedPermit::new(self))
//        } else {
//            drop(state);
//            Err(TrySendError::Full(self))
//        }
//    }
//}

///// The last sender that's dropped will mark the channel as disconnected.
//impl<T> Drop for Sender<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn drop(&mut self) {
//        self.state.drop_sender();
//    }
//}

//#[derive(Debug)]
//struct State<T> {
//    inner: Arc<Mutex<InnerState<T>>>,
//}

//impl<T> Clone for State<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn clone(&self) -> Self {
//        Self {
//            inner: Arc::clone(&self.inner),
//        }
//    }
//}

//impl<T> State<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn inner_mut(&self) -> impl DerefMut<Target = InnerState<T>> + '_ {
//        self.inner.lock().unwrap()
//    }

//    fn next_waker_id(&self) -> WakerId {
//        self.inner_mut().next_waker_id()
//    }

//    fn new_sender(&self) -> Sender<T> {
//        self.inner_mut().senders_count += 1;
//        Sender::new(self.clone())
//    }

//    fn new_receiver(&self) -> Receiver<T> {
//        self.inner_mut().receivers_count += 1;
//        Receiver::new(self.clone())
//    }

//    fn wake_all(wakers: Option<impl Iterator<Item = Waker>>) {
//        if let Some(wakers) = wakers {
//            for waker in wakers {
//                waker.wake()
//            }
//        }
//    }

//    fn close_all_receivers(&self) {
//        let (send_wakers, recv_wakers) = {
//            let mut inner = self.inner_mut();
//            let send_wakers = inner.mark_disconnected_and_take_send_futures();
//            // Keep receivers alive in case there are any active Permits.
//            let recv_wakers = (inner.reserved_count == 0)
//                .then(|| inner.mark_disconnected_and_take_recv_futures());

//            (send_wakers, recv_wakers)
//        };

//        Self::wake_all(Some(send_wakers));
//        Self::wake_all(recv_wakers);
//    }

//    fn drop_sender(&self) {
//        let wakers = {
//            let mut inner = self.inner_mut();
//            inner.senders_count -= 1;

//            (inner.senders_count == 0).then(|| inner.mark_disconnected_and_take_recv_futures())
//        };

//        Self::wake_all(wakers);
//    }

//    fn drop_receiver(&self) {
//        let wakers = {
//            let mut inner = self.inner_mut();
//            inner.receivers_count -= 1;

//            (inner.receivers_count == 0).then(|| inner.mark_disconnected_and_take_send_futures())
//        };

//        Self::wake_all(wakers);
//    }

//    fn drop_permit(&self, has_sent: bool) {
//        let waker = {
//            let mut inner = self.inner_mut();
//            inner.reserved_count -= 1;

//            // When the permit was not used for sending, it means a spot was freed, so we can notify
//            // one of the senders to proceed.
//            (!has_sent)
//                .then(|| inner.waiting_send_futures.pop_first())
//                .flatten()
//        };

//        if let Some((_, waker)) = waker {
//            waker.wake();
//        }
//    }

//    fn drop_send_future(&self, fut: Option<NodeRef<SendWaiting>>) {
//        let waker = {
//            let mut inner = self.inner_mut();

//            if let Some(fut) = fut {
//                inner.waiting_send_futures.remove(fut);
//                inner.reserved_count -= fut.count
//            }

//            // If there's room for the next sender in the queue, wake it.
//            match inner.waiting_send_futures.head() {
//                Some(next) => inner.has_room_for(next.count).then(|| next.waker.clone()),
//                None => None,
//            }
//        };

//        if let Some(waker) = waker {
//            waker.wake();
//        }
//    }

//    fn drop_recv_future(&self, id: &WakerId, has_received: bool) {
//        let waker = {
//            let mut inner = self.inner_mut();
//            let was_awoken = inner.waiting_recv_futures.remove(id).is_none();

//            // Wake another receiver in case this one has been awoken, but it was dropped before it
//            // managed to receive anything.
//            if was_awoken {
//                if has_received {
//                    // If we have received, it means a spot was freed in the internal buffer, so wake
//                    // one sender.
//                    inner.take_one_send_future_waker()
//                } else {
//                    // If we have not received, it means another RecvFuture might take over.
//                    inner.take_one_recv_future_waker()
//                }
//            } else {
//                None
//            }
//        };

//        if let Some(waker) = waker {
//            waker.wake();
//        }
//    }
//}

//struct SendWaiting {
//    count: usize,
//    waker: Waker,
//}

//impl SendWaiting {
//    fn new(count: usize, waker: Waker) -> Self {
//        Self {
//            count,
//            waker
//        }
//    }
//}

//// TODO we need acustom impl of a LinkedList to be able to add/remove wakers, instead of using
//// WakerId.
////
//// Moreover, currently reservations are just a counter, they don't actually take the
//// spot in the buffer, so in the case of a single consumer the result will be
//// re-ordered. We could keep an enum like this:
////
//// ```
//// enum Spot<T> {
////   Value(T),
////   Reserved,
//// }
//// ```
////
//// in the buffer so that we know we need to wait for the reservations.
//struct InnerState<T> {
//    /// Next message ready to be received.
//    next: Option<T>,
//    /// Buffered messages, capacity is fixed.
//    buffer: VecDeque<T>,
//    /// All reservations.
//    reservations: LinkedList<Message<T>>,
//    cap: usize,
//    len: usize,
//    /// Total of created receivers.
//    receivers_count: usize,
//    /// Total of created senders.
//    senders_count: usize,
//    /// Senders waiting to send.
//    waiting_send_futures: LinkedList<SendWaiting>,
//    /// Receivers waiting to receive.
//    waiting_recv_futures: LinkedList<Waker>,
//    /// False by default, true when all senders or all receivers are dropped.
//    disconnected: bool,
//}

//impl<T> InnerState<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn has_room_for(&self, required_num_items: usize) -> bool {
//        let space = self.buffer.capacity() - self.buffer.len();
//        space >= required_num_items
//    }

//    #[must_use]
//    fn send_and_get_receiver_waker(&mut self, value: T, waker: NodeRef<SendWaiting>) -> Option<&Waker> {
//        self.buffer.push_back(value);
//        self.waiting_send_futures.remove(waker);
//        self.waiting_recv_futures.head()
//    }

//    fn mark_disconnected_and_take_send_futures(&mut self) -> impl Iterator<Item = Waker> {
//        self.disconnected = true;
//        Self::take_all_wakers(&mut self.waiting_send_futures).into_values()
//    }

//    fn mark_disconnected_and_take_recv_futures(&mut self) -> impl Iterator<Item = Waker> {
//        self.disconnected = true;
//        Self::take_all_wakers(&mut self.waiting_recv_futures).into_values()
//    }

//    #[must_use]
//    fn take_one_send_future_waker(&mut self) -> Option<Waker> {
//        if self.has_room_for(1) {
//            Self::take_one_waker(&mut self.waiting_send_futures)
//        } else {
//            None
//        }
//    }

//    #[must_use]
//    fn take_one_recv_future_waker(&mut self) -> Option<Waker> {
//        if !self.buffer.is_empty() {
//            Self::take_one_waker(&mut self.waiting_recv_futures)
//        } else {
//            None
//        }
//    }

//    #[must_use]
//    fn take_one_waker(map: &mut BTreeMap<WakerId, Waker>) -> Option<Waker> {
//        map.pop_first().map(|(_, waker)| waker)
//    }

//    fn take_all_wakers(map: &mut BTreeMap<WakerId, Waker>) -> BTreeMap<WakerId, Waker> {
//        std::mem::take(map)
//    }
//}

//enum Message<T> {
//    Value(T),
//    Reserved,
//}

//type WakerId = usize;

//// struct SendFuture<'a, T>
//// where
////     T: Send + Sync + 'static,
//// {
////     sender: &'a Sender<T>,
////     position: Option<NodeRef<Waker>>,
////     value: Option<T>,
//// }

//// impl<'a, T> SendFuture<'a, T>
//// where
////     T: Send + Sync + 'static,
//// {
////     fn new(sender: &'a Sender<T>, value: T) -> Self {
////         Self {
////             sender,
////             position: None,
////             value: Some(value),
////         }
////     }
//// }

//// impl<'a, T> Unpin for SendFuture<'a, T> where T: Send + Sync + 'static {}

//// impl<'a, T> Future for SendFuture<'a, T>
//// where
////     T: Send + Sync + 'static,
//// {
////     type Output = Result<(), SendError<T>>;

////     fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
////         // Already sent this value.
////         if self.value.is_none() {
////             return Poll::Ready(Ok(()));
////         }

////         let this = self.deref_mut();
////         let mut state = this.sender.state.inner_mut();

////         if state.disconnected {
////             Poll::Ready(Err(SendError(this.value.take().expect("some value"))))
////         } else if state.has_room_for(1) {
////             if let Some(waker) = state.send_get_receiver_waker(
////                 this.value.take().expect("some value"),
////                 this.position.take().expect("position"),
////             ) {
////                 drop(state);
////                 waker.wake();
////             }
////             Poll::Ready(Ok(()))
////         } else if self.position.is_none() {
////             let position = state
////                 .waiting_send_futures
////                 .push(cx.waker().clone());
////             self.position = Some(position);
////             Poll::Pending
////         } else {
////             Poll::Pending
////         }
////     }
//// }

//// impl<'a, T> Drop for SendFuture<'a, T>
//// where
////     T: Send + Sync + 'static,
//// {
////     fn drop(&mut self) {
////         let has_sent = self.value.is_none();
////         self.sender.state.drop_send_future(self.position.take());
////     }
//// }

///// Permit holds a spot in the internal buffer so the message can be sent without awaiting.
//pub struct Permit<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    sender: &'a Sender<T>,
//    node_ref: NodeRef<Message<T>>,
//    has_sent: bool,
//}

//impl<'a, T> Permit<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    fn new(sender: &'a Sender<T>) -> Self {
//        Self {
//            sender,
//            has_sent: false,
//        }
//    }

//    /// Writes a message to the internal buffer.
//    pub fn send(mut self, value: T) {
//        if let Some(waker) = self.sender.state.inner_mut().send_and_get_waker(value) {
//            waker.wake();
//        }
//        self.has_sent = true;
//        drop(self)
//    }
//}

//impl<'a, T> Drop for Permit<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    fn drop(&mut self) {
//        self.sender.state.drop_permit(self.has_sent);
//    }
//}

//pub struct PermitIterator<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    sender: &'a Sender<T>,
//    count: usize,
//}

//impl<'a, T> PermitIterator<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    pub fn new(sender: &'a Sender<T>, count: usize) -> Self {
//        Self { sender, count }
//    }
//}

//impl<'a, T> Iterator for PermitIterator<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    type Item = Permit<'a, T>;

//    fn next(&mut self) -> Option<Self::Item> {
//        if self.count == 0 {
//            None
//        } else {
//            self.count -= 1;
//            Some(Permit::new(self.sender))
//        }
//    }
//}

//impl<'a, T> Drop for PermitIterator<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    fn drop(&mut self) {
//        for _ in 0..self.count {
//            self.sender.state.drop_permit(false);
//        }
//    }
//}

//pub struct OwnedPermit<T>
//where
//    T: Send + Sync + 'static,
//{
//    sender: Option<Sender<T>>,
//}

//impl<T> OwnedPermit<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn new(sender: Sender<T>) -> Self {
//        Self {
//            sender: Some(sender),
//        }
//    }

//    /// Writes a message to the internal buffer.
//    pub fn send(mut self, value: T) -> Sender<T> {
//        let sender = self
//            .sender
//            .take()
//            .expect("sender is only taken when permit is consumed");

//        if let Some(waker) = sender.state.inner_mut().send_and_get_waker(value) {
//            waker.wake();
//        }

//        sender.state.drop_permit(true);

//        sender
//    }

//    pub fn release(mut self) -> Sender<T> {
//        let sender = self
//            .sender
//            .take()
//            .expect("sender is only taken when permit is consumed");

//        sender.state.drop_permit(false);

//        sender
//    }
//}

//impl<T> Drop for OwnedPermit<T>
//where
//    T: Send + Sync + 'static,
//{
//    fn drop(&mut self) {
//        // if we haven't called send or release:
//        if let Some(sender) = &self.sender {
//            sender.state.drop_permit(false);
//        }
//    }
//}

//struct ReserveFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    id: WakerId,
//    waiting: Option<NodeRef<SendWaiting>>,
//    state: &'a State<T>,
//    count: usize,
//}

//impl<'a, T> ReserveFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    fn new(id: WakerId, state: &'a State<T>, count: usize) -> Self {
//        Self {
//            id,
//            state,
//            count,
//            has_reserved: false,
//        }
//    }
//}

//impl<'a, T> Future for ReserveFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    type Output = Result<(), ReserveError>;

//    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
//        if self.has_reserved {
//            panic!("ReservedFuture polled after ready");
//        }

//        let this = self.deref_mut();
//        let mut state = this.state.inner_mut();
//        if state.disconnected {
//            Poll::Ready(Err(ReserveError))
//        } else if state.has_room_for(this.count) {
//            state.buffer.push(Message::Reserved);
//            state.reserved_count += this.count;
//            Poll::Ready(Ok(()))
//        } else if self.waiting.is_none() {
//            let waiting = state
//                .waiting_send_futures
//                .push(SendWaiting::new(self.count, cx.waker().clone()));
//            self.waiting = Some(waiting);
//            Poll::Pending
//        } else {
//            Poll::Pending
//        }
//    }
//}

//struct RecvFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    receiver: &'a Receiver<T>,
//    waker_ref: NodeRef<Waker>,
//    has_received: bool,
//}

//impl<'a, T> RecvFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    fn new(receiver: &'a Receiver<T>) -> Self {
//        Self {
//            receiver,
//            has_received: false,
//        }
//    }
//}

//impl<'a, T> Unpin for RecvFuture<'a, T> where T: Send + Sync + 'static {}

//impl<'a, T> Future for RecvFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    type Output = Result<T, RecvError>;

//    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
//        let this = self.deref_mut();
//        let mut state = this.receiver.state.inner_mut();
//        let value = state.buffer.pop_front();

//        match value {
//            Some(value) => {
//                this.has_received = true;
//                Poll::Ready(Ok(value))
//            }
//            None => {
//                if state.disconnected && state.reserved_count == 0 {
//                    Poll::Ready(Err(RecvError))
//                } else {
//                    state.waiting_recv_futures.push(cx.waker().clone());
//                    if let Some(waker) = state.take_one_send_future_waker() {
//                        drop(state);
//                        waker.wake();
//                    }
//                    Poll::Pending
//                }
//            }
//        }
//    }
//}

//impl<'a, T> Drop for RecvFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    fn drop(&mut self) {
//        self.receiver
//            .state
//            .drop_recv_future(&self.id, self.has_received);
//    }
//}

//struct RecvManyFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    id: WakerId,
//    receiver: &'a Receiver<T>,
//    vec: &'a mut Vec<T>,
//    count: usize,
//    has_received: bool,
//}

//impl<'a, T> RecvManyFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    fn new(id: WakerId, receiver: &'a Receiver<T>, vec: &'a mut Vec<T>, count: usize) -> Self {
//        Self {
//            id,
//            receiver,
//            vec,
//            count,
//            has_received: false,
//        }
//    }
//}

//impl<'a, T> Unpin for RecvManyFuture<'a, T> where T: Send + Sync + 'static {}

//impl<'a, T> Future for RecvManyFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    type Output = Result<usize, RecvError>;

//    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
//        let this = self.deref_mut();
//        let mut state = this.receiver.state.inner_mut();

//        let value = state.buffer.pop_front();

//        match value {
//            Some(value) => {
//                let mut num_received = 1;
//                this.vec.push(value);

//                for _ in 1..this.count {
//                    match state.buffer.pop_front() {
//                        Some(value) => {
//                            this.vec.push(value);
//                            num_received += 1;
//                        }
//                        None => break,
//                    }
//                }

//                this.has_received = true;
//                Poll::Ready(Ok(num_received))
//            }
//            None => {
//                if state.disconnected && state.reserved_count == 0 {
//                    Poll::Ready(Err(RecvError))
//                } else {
//                    state
//                        .waiting_recv_futures
//                        .insert(this.id, cx.waker().clone());
//                    if let Some(waker) = state.take_one_send_future_waker() {
//                        drop(state);
//                        waker.wake();
//                    }
//                    Poll::Pending
//                }
//            }
//        }
//    }
//}

//impl<'a, T> Drop for RecvManyFuture<'a, T>
//where
//    T: Send + Sync + 'static,
//{
//    fn drop(&mut self) {
//        self.receiver
//            .state
//            .drop_recv_future(&self.id, self.has_received);
//    }
//}

//#[cfg(test)]
//mod testing {
//    use std::collections::BTreeSet;
//    use std::time::Duration;

//    use super::*;

//    #[tokio::test]
//    async fn send_receive() {
//        let (tx, rx) = channel(1);
//        tx.send(1).await.expect("no error");
//        let res = rx.recv().await.expect("no error");
//        assert_eq!(res, 1);
//    }

//    #[tokio::test]
//    async fn mpsc() {
//        let (tx, rx) = channel(1);

//        let num_workers = 10;
//        let count = 10;
//        let mut tasks = Vec::with_capacity(num_workers);

//        for i in 0..num_workers {
//            let tx = tx.clone();
//            let task = tokio::spawn(async move {
//                for j in 0..count {
//                    let val = i * count + j;
//                    tx.send(val).await.expect("Failed to send");
//                }
//            });
//            tasks.push(task);
//        }

//        let total = count * num_workers;
//        let mut values = BTreeSet::new();

//        for _ in 0..total {
//            let value = rx.recv().await.expect("no error");
//            values.insert(value);
//        }

//        let exp = (0..total).collect::<Vec<_>>();
//        let got = values.into_iter().collect::<Vec<_>>();
//        assert_eq!(exp, got);

//        for task in tasks {
//            task.await.expect("failed to join task");
//        }
//    }

//    async fn run_tasks<F, Fut>(send: F)
//    where
//        Fut: Future<Output = Sender<usize>> + Send,
//        F: Send + Sync + 'static + Copy,
//        F: Fn(Sender<usize>, usize) -> Fut,
//    {
//        let (tx, rx) = channel(1);

//        let num_workers = 10;
//        let count = 10;
//        let mut tasks = Vec::with_capacity(num_workers);

//        for i in 0..num_workers {
//            let mut tx = tx.clone();
//            let task = tokio::spawn(async move {
//                for j in 0..count {
//                    let val = i * count + j;
//                    tx = send(tx, val).await;
//                }
//            });
//            tasks.push(task);
//        }

//        let total = count * num_workers;
//        let values = Arc::new(Mutex::new(BTreeSet::new()));

//        for _ in 0..num_workers {
//            let values = values.clone();
//            let rx = rx.clone();
//            let task = tokio::spawn(async move {
//                for _ in 0..count {
//                    let val = rx.recv().await.expect("Failed to recv");
//                    values.lock().unwrap().insert(val);
//                }
//            });
//            tasks.push(task);
//        }

//        for task in tasks {
//            task.await.expect("failed to join task");
//        }

//        let exp = (0..total).collect::<Vec<_>>();
//        let got = std::mem::take(values.lock().unwrap().deref_mut())
//            .into_iter()
//            .collect::<Vec<_>>();
//        assert_eq!(exp, got);
//    }

//    #[tokio::test]
//    async fn mpmc_multiple_tasks() {
//        run_tasks(|tx, value| async move {
//            tx.send(value).await.expect("Failed to send");
//            tx
//        })
//        .await;
//    }

//    #[tokio::test]
//    async fn mpmc_reserve() {
//        run_tasks(|tx, value| async move {
//            tx.reserve().await.expect("Failed to send").send(value);
//            tx
//        })
//        .await;
//    }

//    #[tokio::test]
//    async fn mpmc_try_reserve() {
//        run_tasks(|tx, value| async move {
//            loop {
//                match tx.try_reserve() {
//                    Ok(permit) => {
//                        permit.send(value);
//                    }
//                    Err(_err) => {
//                        tokio::time::sleep(Duration::ZERO).await;
//                        continue;
//                    }
//                };

//                return tx;
//            }
//        })
//        .await;
//    }

//    #[tokio::test]
//    async fn send_errors() {
//        let (tx, rx) = channel::<i32>(2);
//        assert_eq!(tx.send(1).await, Ok(()));
//        assert_eq!(tx.send(2).await, Ok(()));
//        let task = tokio::spawn({
//            let tx = tx.clone();
//            async move { tx.send(3).await }
//        });
//        drop(rx);
//        assert_eq!(tx.send(4).await, Err(SendError(4)));
//        assert_eq!(task.await.expect("panic"), Err(SendError(3)));
//    }

//    #[test]
//    fn try_send_errors() {
//        let (tx, rx) = channel::<i32>(2);
//        assert_eq!(tx.try_send(1), Ok(()));
//        assert_eq!(tx.try_send(2), Ok(()));
//        assert_eq!(tx.try_send(3), Err(TrySendError::Full(3)));
//        assert_eq!(tx.try_send(4), Err(TrySendError::Full(4)));
//        drop(rx);
//        assert_eq!(tx.try_send(5), Err(TrySendError::Disconnected(5)));
//    }

//    #[tokio::test]
//    async fn reserve_errors() {
//        let (tx, rx) = channel::<i32>(2);
//        tx.reserve().await.expect("reserved 1");
//        tx.reserve().await.expect("reserved 2");
//        let task = tokio::spawn({
//            let tx = tx.clone();
//            async move {
//                assert!(matches!(tx.reserve().await, Err(ReserveError)));
//            }
//        });
//        drop(rx);
//        assert!(matches!(tx.reserve().await, Err(ReserveError)));
//        task.await.expect("no panic");
//    }

//    #[test]
//    fn try_reserve_errors() {
//        let (tx, rx) = channel::<i32>(2);
//        let _res1 = tx.try_reserve().expect("reserved 1");
//        let _res2 = tx.try_reserve().expect("reserved 2");
//        assert!(matches!(tx.try_reserve(), Err(TryReserveError::Full)));
//        assert!(matches!(tx.try_reserve(), Err(TryReserveError::Full)));
//        drop(rx);
//        assert!(matches!(
//            tx.try_reserve(),
//            Err(TryReserveError::Disconnected)
//        ));
//    }

//    #[tokio::test]
//    async fn recv_future_awoken_but_unused() {
//        let (tx, rx) = channel::<i32>(1);
//        let mut recv = Box::pin(rx.recv());
//        let rx2 = rx.clone();
//        // Try receiving from rx2, but don't drop it yet.
//        tokio::select! {
//            biased;
//            _ = &mut recv => {
//                panic!("unexpected recv");
//            }
//            _ = ReadyFuture {} => {}
//        }
//        let task = tokio::spawn(async move { rx2.recv().await });
//        // Yield the current task so task above can be started.
//        tokio::time::sleep(Duration::ZERO).await;
//        tx.try_send(1).expect("sent");
//        // It would hang without the drop, since the recv would be awoken, but we'd never await for
//        // it. This is the main flaw of this design where only a single future is awoken at the
//        // time. Alternatively, we could wake all of them at once, but this would most likely
//        // result in performance degradation due to lock contention.
//        drop(recv);
//        let res = task.await.expect("no panic").expect("receivd");
//        assert_eq!(res, 1);
//    }

//    #[tokio::test]
//    async fn try_reserve_unused_permit_and_send() {
//        let (tx, rx) = channel::<i32>(1);
//        let permit = tx.try_reserve().expect("reserved");
//        let task = tokio::spawn({
//            let tx = tx.clone();
//            async move { tx.send(1).await }
//        });
//        drop(permit);
//        task.await.expect("no panic").expect("sent");
//        assert_eq!(rx.try_recv().expect("recv"), 1);
//    }

//    #[tokio::test]
//    async fn try_reserve_unused_permit_and_other_permit() {
//        let (tx, rx) = channel::<i32>(1);
//        let permit = tx.try_reserve().expect("reserved");
//        let task = tokio::spawn({
//            let tx = tx.clone();
//            async move { tx.reserve().await.expect("reserved").send(1) }
//        });
//        drop(permit);
//        task.await.expect("no panic");
//        assert_eq!(rx.try_recv().expect("recv"), 1);
//    }

//    #[tokio::test]
//    async fn receiver_close_all() {
//        let (tx, rx1) = channel::<i32>(3);
//        let rx2 = rx1.clone();
//        let permit1 = tx.reserve().await.unwrap();
//        let permit2 = tx.reserve().await.unwrap();
//        tx.send(1).await.unwrap();
//        rx1.close_all();
//        assert_eq!(rx1.recv().await.unwrap(), 1);
//        assert_no_recv(&rx1).await;
//        assert_no_recv(&rx2).await;
//        permit1.send(2);
//        permit2.send(3);
//        assert_eq!(rx1.recv().await.unwrap(), 2);
//        assert_eq!(rx2.try_recv().unwrap(), 3);
//        assert_eq!(rx1.recv().await, Err(RecvError));
//        assert_eq!(rx2.recv().await, Err(RecvError));
//        assert!(matches!(tx.send(3).await, Err(SendError(3))));
//    }

//    #[tokio::test]
//    async fn receiver_close_all_permit_drop() {
//        let (tx, rx) = channel::<i32>(3);
//        let permit = tx.reserve().await.unwrap();
//        rx.close_all();
//        assert_no_recv(&rx).await;
//        drop(permit);
//        assert_eq!(rx.recv().await, Err(RecvError));
//    }

//    #[tokio::test]
//    async fn reserve_owned() {
//        let (tx, rx) = channel::<usize>(4);
//        let tx = tx.reserve_owned().await.unwrap().send(1);
//        let tx = tx.reserve_owned().await.unwrap().send(2);
//        let tx = tx.try_reserve_owned().unwrap().send(3);
//        let tx = tx.try_reserve_owned().unwrap().release();
//        let tx = tx.try_reserve_owned().unwrap().send(4);
//        assert!(matches!(
//            tx.clone().try_reserve_owned(),
//            Err(TrySendError::Full(_))
//        ));
//        for i in 1..=4 {
//            assert_eq!(rx.try_recv().unwrap(), i);
//        }
//        drop(rx);
//        assert!(matches!(
//            tx.clone().reserve_owned().await,
//            Err(ReserveError)
//        ));
//        assert!(matches!(
//            tx.try_reserve_owned(),
//            Err(TrySendError::Disconnected(_))
//        ));
//    }

//    #[tokio::test]
//    async fn reserve_many() {
//        let (tx, rx) = channel::<usize>(10);
//        let p1 = tx.reserve_many(5).await.unwrap();
//        let p2 = tx.try_reserve_many(5).unwrap();
//        assert!(matches!(tx.try_send(11), Err(TrySendError::Full(11))));
//        for (i, p) in p1.enumerate() {
//            p.send(i);
//        }
//        for (i, p) in p2.enumerate() {
//            p.send(i + 5);
//        }
//        for i in 0..10 {
//            assert_eq!(rx.try_recv().unwrap(), i);
//        }
//    }

//    #[tokio::test]
//    async fn reserve_many_drop() {
//        let (tx, _rx) = channel::<usize>(2);
//        let it = tx.reserve_many(2).await.unwrap();
//        drop(it);
//        tx.try_send(1).unwrap();
//        tx.try_send(2).unwrap();
//        assert!(matches!(tx.try_send(3), Err(TrySendError::Full(3))));
//    }

//    #[tokio::test]
//    async fn reserve_many_drop_halfway() {
//        let (tx, _rx) = channel::<usize>(4);
//        let mut it = tx.reserve_many(4).await.unwrap();
//        it.next().unwrap().send(1);
//        it.next().unwrap().send(2);
//        drop(it);
//        tx.try_send(3).unwrap();
//        tx.try_send(4).unwrap();
//        assert!(matches!(tx.try_send(5), Err(TrySendError::Full(5))));
//    }

//    async fn assert_no_recv<T>(rx: &Receiver<T>)
//    where
//        T: std::fmt::Debug + Send + Sync + 'static,
//    {
//        tokio::select! {
//            result = rx.recv() => {
//                panic!("unexpected recv: {result:?}");
//            },
//            _ = tokio::time::sleep(std::time::Duration::ZERO) => {},
//        }
//    }

//    struct ReadyFuture {}

//    impl Future for ReadyFuture {
//        type Output = ();

//        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
//            Poll::Ready(())
//        }
//    }
//}
