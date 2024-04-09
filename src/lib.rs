//! A multi-producer, multi-consumer async channel with reservations.
//!
//! Example usage:
//!
//! ```rust
//! tokio_test::block_on(async {
//!     let (tx1, rx1) = mpmc_async::channel(2);
//!
//!     let task = tokio::spawn(async move {
//!       let rx2 = rx1.clone();
//!       assert_eq!(rx1.recv().await.unwrap(), 2);
//!       assert_eq!(rx2.recv().await.unwrap(), 1);
//!     });
//!
//!     let tx2 = tx1.clone();
//!     let permit = tx1.reserve().await.unwrap();
//!     tx2.send(1).await.unwrap();
//!     permit.send(2);
//!
//!     task.await.unwrap();
//! });
//! ```
//!
//! A more complex example with multiple sender and receiver tasks:
//!
//! ```rust
//! use std::collections::BTreeSet;
//! use std::ops::DerefMut;
//! use std::sync::{Arc, Mutex};
//!
//! tokio_test::block_on(async {
//!     let (tx, rx) = mpmc_async::channel(1);
//!
//!     let num_workers = 10;
//!     let count = 10;
//!     let mut tasks = Vec::with_capacity(num_workers);
//!
//!     for i in 0..num_workers {
//!         let mut tx = tx.clone();
//!         let task = tokio::spawn(async move {
//!             for j in 0..count {
//!                 let val = i * count + j;
//!                 tx.reserve().await.expect("no error").send(val);
//!             }
//!         });
//!         tasks.push(task);
//!     }
//!
//!     let total = count * num_workers;
//!     let values = Arc::new(Mutex::new(BTreeSet::new()));
//!
//!     for _ in 0..num_workers {
//!         let values = values.clone();
//!         let rx = rx.clone();
//!         let task = tokio::spawn(async move {
//!             for _ in 0..count {
//!                 let val = rx.recv().await.expect("Failed to recv");
//!                 values.lock().unwrap().insert(val);
//!             }
//!         });
//!         tasks.push(task);
//!     }
//!
//!     for task in tasks {
//!         task.await.expect("failed to join task");
//!     }
//!
//!     let exp = (0..total).collect::<Vec<_>>();
//!     let got = std::mem::take(values.lock().unwrap().deref_mut())
//!         .into_iter()
//!         .collect::<Vec<_>>();
//!     assert_eq!(exp, got);
//! });
//! ```
mod linked_list;
mod queue;
mod state;

use std::fmt::{Debug, Display};
use std::future::Future;
use std::ops::DerefMut;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use self::linked_list::NodeRef;
use self::queue::Spot;
use self::state::SendWaker;
use self::state::State;

/// Occurs when all receivers have been dropped.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct SendError<T>(pub T);

impl<T> Display for SendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Send failed: disconnected")
    }
}

/// TrySendError occurs when the channel is empty or all receivers have been dropped.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TrySendError<T> {
    /// Channel full
    Full(T),
    /// Disconnected
    Disconnected(T),
}

impl<T> Display for TrySendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrySendError::Full(_) => f.write_str("Try send failed: full"),
            TrySendError::Disconnected(_) => f.write_str("Try send failed: disconnected"),
        }
    }
}

/// Occurs when all receivers have been dropped.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct ReserveError;

impl Display for ReserveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Reserve failed: disconnected")
    }
}

/// Occurs when the channel is full, or all receivers have been dropped.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TryReserveError {
    /// Channel full
    Full,
    /// Disconnected
    Disconnected,
}

impl Display for TryReserveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TryReserveError::Full => f.write_str("Try send failed: full"),
            TryReserveError::Disconnected => f.write_str("Try send failed: disconnected"),
        }
    }
}

/// Occurs when all senders have been dropped.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct RecvError;

impl Display for RecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Recv failed: disconnected")
    }
}

/// Occurs when channel is empty or all senders have been dropped.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TryRecvError {
    /// Channel is empty
    Empty,
    /// Disconnected
    Disconnected,
}

impl Display for TryRecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TryRecvError::Empty => f.write_str("Try recv failed: empty"),
            TryRecvError::Disconnected => f.write_str("Try recv failed: disconnected"),
        }
    }
}

/// Creates a new bounded channel. When `cap` is 0 it will be increased to 1.
pub fn channel<T>(mut cap: usize) -> (Sender<T>, Receiver<T>) {
    if cap == 0 {
        cap = 1
    }

    let state = State::new(cap);

    (state.new_sender(), state.new_receiver())
}

/// Receives messages sent by [Sender].
pub struct Receiver<T> {
    state: State<T>,
}

/// Cloning creates new a instance with the shared state  and increases the internal reference
/// counter. It is guaranteed that a single message will be distributed to exacly one receiver
/// future awaited after calling `recv()`.
impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.state.new_receiver()
    }
}

impl<T> Receiver<T> {
    fn new(state: State<T>) -> Self {
        Self { state }
    }

    /// Disconnects all receivers from senders. The receivers will still receive all buffered
    /// or reserved messages before returning an error, allowing a graceful teardown.
    pub fn close_all(&self) {
        self.state.close_all_receivers();
    }

    /// Waits until there's a message to be read and returns. Returns an error when there are no
    /// more messages in the queue and all [Sender]s have been dropped.
    pub async fn recv(&self) -> Result<T, RecvError> {
        let recv = RecvFuture::new(self);
        recv.await
    }

    /// Checks if there's a message to be read and returns immediately. Returns an error when the
    /// channel is disconnected or empty.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.state.try_recv()
    }

    pub async fn recv_many(&self, vec: &mut Vec<T>, count: usize) -> Result<usize, RecvError> {
        let recv = RecvManyFuture::new(self, vec, count);
        recv.await
    }

    pub fn try_recv_many(&self, vec: &mut Vec<T>, count: usize) -> Result<usize, TryRecvError> {
        self.state.try_recv_many(vec, count)
    }
}

/// The last reciever that's dropped will mark the channel as disconnected.
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.state.drop_receiver();
    }
}

/// Producers messages to be read by [Receiver]s.
pub struct Sender<T> {
    state: State<T>,
}

impl<T> Debug for Sender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sender")
    }
}

/// Cloning creates new a instance with the shared state  and increases the internal reference
/// counter.
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.state.new_sender()
    }
}

impl<T> Sender<T> {
    fn new(state: State<T>) -> Sender<T> {
        Self { state }
    }

    /// Waits until the value is sent or returns an when all receivers have been dropped.
    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
        SendFuture::new(&self.state, value).await
    }

    /// Sends without blocking or returns an when all receivers have been dropped.
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        self.state.try_send(value)
    }

    /// Waits until a permit is reserved or returns an when all receivers have been dropped.
    pub async fn reserve(&self) -> Result<Permit<'_, T>, ReserveError> {
        let reservation = ReserveFuture::new(&self.state, 1).await?;
        Ok(Permit::new(self, reservation))
    }

    /// Reserves a permit or returns an when all receivers have been dropped.
    pub fn try_reserve(&self) -> Result<Permit<'_, T>, TryReserveError> {
        let reservation = self.state.try_reserve(1)?;
        Ok(Permit::new(self, reservation))
    }

    /// Waits until multiple Permits in the queue are reserved.
    pub async fn reserve_many(&self, count: usize) -> Result<PermitIterator<'_, T>, ReserveError> {
        let reservation = ReserveFuture::new(&self.state, count).await?;
        Ok(PermitIterator::new(self, reservation, count))
    }

    /// Reserves multiple Permits in the queue, or errors out when there's no room.
    pub fn try_reserve_many(&self, count: usize) -> Result<PermitIterator<'_, T>, TryReserveError> {
        let reservation = self.state.try_reserve(count)?;
        Ok(PermitIterator::new(self, reservation, count))
    }

    /// Like [Sender::reserve], but takes ownership of `Sender` until sending is done.
    pub async fn reserve_owned(self) -> Result<OwnedPermit<T>, ReserveError> {
        let reservation = ReserveFuture::new(&self.state, 1).await?;
        Ok(OwnedPermit::new(self, reservation))
    }

    /// Reserves a permit or returns an when all receivers have been dropped.
    pub fn try_reserve_owned(self) -> Result<OwnedPermit<T>, TrySendError<Self>> {
        let reservation = match self.state.try_reserve(1) {
            Ok(reservation) => reservation,
            Err(TryReserveError::Disconnected) => return Err(TrySendError::Disconnected(self)),
            Err(TryReserveError::Full) => return Err(TrySendError::Full(self)),
        };
        Ok(OwnedPermit::new(self, reservation))
    }
}

/// The last sender that's dropped will mark the channel as disconnected.
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.state.drop_sender();
    }
}

/// Permit holds a spot in the internal buffer so the message can be sent without awaiting.
pub struct Permit<'a, T> {
    sender: &'a Sender<T>,
    reservation: Option<NodeRef<Spot<T>>>,
}

impl<'a, T> Permit<'a, T> {
    fn new(sender: &'a Sender<T>, reservation: NodeRef<Spot<T>>) -> Self {
        Self {
            sender,
            reservation: Some(reservation),
        }
    }

    /// Writes a message to the internal buffer.
    pub fn send(mut self, value: T) {
        let reservation = self.reservation.take().expect("reservation");
        self.sender.state.send_with_permit(reservation, value);
    }
}

impl<'a, T> Drop for Permit<'a, T> {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            self.sender.state.drop_permit(reservation, 1);
        }
    }
}

pub struct PermitIterator<'a, T> {
    sender: &'a Sender<T>,
    reservation: Option<NodeRef<Spot<T>>>,
    count: usize,
}

impl<'a, T> PermitIterator<'a, T> {
    pub fn new(sender: &'a Sender<T>, reservation: NodeRef<Spot<T>>, count: usize) -> Self {
        Self {
            sender,
            reservation: Some(reservation),
            count,
        }
    }
}

impl<'a, T> Iterator for PermitIterator<'a, T> {
    type Item = Permit<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.count == 0 {
            None
        } else {
            self.count -= 1;

            let reservation = if self.count == 0 {
                self.reservation.take().expect("reservation")
            } else {
                self.reservation.clone().expect("reservation")
            };

            Some(Permit::new(self.sender, reservation))
        }
    }
}

impl<'a, T> Drop for PermitIterator<'a, T> {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            self.sender.state.drop_permit(reservation, self.count);
        }
    }
}

pub struct OwnedPermit<T> {
    sender_and_reservation: Option<(Sender<T>, NodeRef<Spot<T>>)>,
}

impl<T> OwnedPermit<T> {
    fn new(sender: Sender<T>, reservation: NodeRef<Spot<T>>) -> Self {
        Self {
            sender_and_reservation: Some((sender, reservation)),
        }
    }

    /// Writes a message to the internal buffer.
    pub fn send(mut self, value: T) -> Sender<T> {
        let (sender, reservation) = self
            .sender_and_reservation
            .take()
            .expect("sender and reservation");

        sender.state.send_with_permit(reservation, value);

        sender
    }

    pub fn release(mut self) -> Sender<T> {
        let (sender, reservation) = self
            .sender_and_reservation
            .take()
            .expect("sender and reservation");

        sender.state.drop_permit(reservation, 1);

        sender
    }
}

impl<T> Drop for OwnedPermit<T> {
    fn drop(&mut self) {
        // if we haven't called send or release:
        if let Some((sender, reservation)) = self.sender_and_reservation.take() {
            sender.state.drop_permit(reservation, 1);
        }
    }
}

struct SendFuture<'a, T> {
    state: &'a State<T>,
    value: Option<T>,
    waiting: Option<NodeRef<SendWaker>>,
}

impl<'a, T> Unpin for SendFuture<'a, T> {}

impl<'a, T> SendFuture<'a, T> {
    fn new(state: &'a State<T>, value: T) -> Self {
        Self {
            state,
            value: Some(value),
            waiting: None,
        }
    }
}

impl<'a, T> Future for SendFuture<'a, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.deref_mut();

        this.state.send(&mut this.value, cx, &mut this.waiting)
    }
}

impl<'a, T> Drop for SendFuture<'a, T> {
    fn drop(&mut self) {
        self.state
            .drop_send_future(&mut self.value, &mut self.waiting)
    }
}

struct ReserveFuture<'a, T> {
    state: &'a State<T>,
    count: usize,
    waiting: Option<NodeRef<SendWaker>>,
}

impl<'a, T> ReserveFuture<'a, T> {
    fn new(state: &'a State<T>, count: usize) -> Self {
        Self {
            state,
            count,
            waiting: None,
        }
    }
}

impl<'a, T> Future for ReserveFuture<'a, T> {
    type Output = Result<NodeRef<Spot<T>>, ReserveError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.deref_mut();

        this.state.reserve(this.count, cx, &mut this.waiting)
    }
}

impl<'a, T> Drop for ReserveFuture<'a, T> {
    fn drop(&mut self) {
        self.state.drop_reserve_future(&mut self.waiting)
    }
}

struct RecvFuture<'a, T> {
    receiver: &'a Receiver<T>,
    waker_ref: Option<NodeRef<Waker>>,
    has_received: bool,
}

impl<'a, T> RecvFuture<'a, T> {
    fn new(receiver: &'a Receiver<T>) -> Self {
        Self {
            receiver,
            waker_ref: None,
            has_received: false,
        }
    }
}

impl<'a, T> Unpin for RecvFuture<'a, T> {}

impl<'a, T> Future for RecvFuture<'a, T> {
    type Output = Result<T, RecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.deref_mut();
        this.receiver
            .state
            .recv(cx, &mut this.has_received, &mut this.waker_ref)
    }
}

impl<'a, T> Drop for RecvFuture<'a, T> {
    fn drop(&mut self) {
        self.receiver
            .state
            .drop_recv_future(self.has_received, &mut self.waker_ref);
    }
}

struct RecvManyFuture<'a, T> {
    receiver: &'a Receiver<T>,
    vec: &'a mut Vec<T>,
    count: usize,
    waker_ref: Option<NodeRef<Waker>>,
    has_received: bool,
}

impl<'a, T> RecvManyFuture<'a, T> {
    fn new(receiver: &'a Receiver<T>, vec: &'a mut Vec<T>, count: usize) -> Self {
        Self {
            receiver,
            vec,
            count,
            waker_ref: None,
            has_received: false,
        }
    }
}

impl<'a, T> Unpin for RecvManyFuture<'a, T> {}

impl<'a, T> Future for RecvManyFuture<'a, T> {
    type Output = Result<usize, RecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.deref_mut();
        this.receiver.state.recv_many(
            cx,
            &mut this.has_received,
            &mut this.waker_ref,
            this.vec,
            this.count,
        )
    }
}

impl<'a, T> Drop for RecvManyFuture<'a, T> {
    fn drop(&mut self) {
        self.receiver
            .state
            .drop_recv_future(self.has_received, &mut self.waker_ref);
    }
}

#[cfg(test)]
mod testing {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn send_receive() {
        let (tx, rx) = channel(1);
        tx.send(1).await.expect("no error");
        let res = rx.recv().await.expect("no error");
        assert_eq!(res, 1);
    }

    #[tokio::test]
    async fn mpsc() {
        let (tx, rx) = channel(1);

        let num_workers = 10;
        let count = 10;
        let mut tasks = Vec::with_capacity(num_workers);

        for i in 0..num_workers {
            let tx = tx.clone();
            let task = tokio::spawn(async move {
                for j in 0..count {
                    let val = i * count + j;
                    tx.send(val).await.expect("Failed to send");
                }
            });
            tasks.push(task);
        }

        let total = count * num_workers;
        let mut values = BTreeSet::new();

        for _ in 0..total {
            let value = rx.recv().await.expect("no error");
            values.insert(value);
        }

        let exp = (0..total).collect::<Vec<_>>();
        let got = values.into_iter().collect::<Vec<_>>();
        assert_eq!(exp, got);

        for task in tasks {
            task.await.expect("failed to join task");
        }
    }

    async fn run_tasks<F, Fut>(send: F)
    where
        Fut: Future<Output = Sender<usize>> + Send,
        F: Send + Sync + 'static + Copy,
        F: Fn(Sender<usize>, usize) -> Fut,
    {
        let (tx, rx) = channel(1);

        let num_workers = 10;
        let count = 10;
        let mut tasks = Vec::with_capacity(num_workers);

        for i in 0..num_workers {
            let mut tx = tx.clone();
            let task = tokio::spawn(async move {
                for j in 0..count {
                    let val = i * count + j;
                    tx = send(tx, val).await;
                }
            });
            tasks.push(task);
        }

        let total = count * num_workers;
        let values = Arc::new(Mutex::new(BTreeSet::new()));

        for _ in 0..num_workers {
            let values = values.clone();
            let rx = rx.clone();
            let task = tokio::spawn(async move {
                for _ in 0..count {
                    let val = rx.recv().await.expect("Failed to recv");
                    values.lock().unwrap().insert(val);
                }
            });
            tasks.push(task);
        }

        for task in tasks {
            task.await.expect("failed to join task");
        }

        let exp = (0..total).collect::<Vec<_>>();
        let got = std::mem::take(values.lock().unwrap().deref_mut())
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(exp, got);
    }

    #[tokio::test]
    async fn mpmc_multiple_tasks() {
        run_tasks(|tx, value| async move {
            tx.send(value).await.expect("Failed to send");
            tx
        })
        .await;
    }

    #[tokio::test]
    async fn mpmc_reserve() {
        run_tasks(|tx, value| async move {
            tx.reserve().await.expect("Failed to send").send(value);
            tx
        })
        .await;
    }

    #[tokio::test]
    async fn mpmc_try_reserve() {
        run_tasks(|tx, value| async move {
            loop {
                match tx.try_reserve() {
                    Ok(permit) => {
                        permit.send(value);
                    }
                    Err(_err) => {
                        tokio::time::sleep(Duration::ZERO).await;
                        continue;
                    }
                };

                return tx;
            }
        })
        .await;
    }

    #[tokio::test]
    async fn send_errors() {
        let (tx, rx) = channel::<i32>(2);
        assert_eq!(tx.send(1).await, Ok(()));
        assert_eq!(tx.send(2).await, Ok(()));
        let task = tokio::spawn({
            let tx = tx.clone();
            async move { tx.send(3).await }
        });
        drop(rx);
        assert_eq!(tx.send(4).await, Err(SendError(4)));
        assert_eq!(task.await.expect("panic"), Err(SendError(3)));
    }

    #[test]
    fn try_send_errors() {
        let (tx, rx) = channel::<i32>(2);
        assert_eq!(tx.try_send(1), Ok(()));
        assert_eq!(tx.try_send(2), Ok(()));
        assert_eq!(tx.try_send(3), Err(TrySendError::Full(3)));
        assert_eq!(tx.try_send(4), Err(TrySendError::Full(4)));
        drop(rx);
        assert_eq!(tx.try_send(5), Err(TrySendError::Disconnected(5)));
    }

    #[tokio::test]
    async fn reserve_errors() {
        let (tx, rx) = channel::<i32>(2);
        tx.reserve().await.expect("reserved 1");
        tx.reserve().await.expect("reserved 2");
        let task = tokio::spawn({
            let tx = tx.clone();
            async move {
                assert!(matches!(tx.reserve().await, Err(ReserveError)));
            }
        });
        drop(rx);
        assert!(matches!(tx.reserve().await, Err(ReserveError)));
        task.await.expect("no panic");
    }

    #[test]
    fn try_reserve_errors() {
        let (tx, rx) = channel::<i32>(2);
        let _res1 = tx.try_reserve().expect("reserved 1");
        let _res2 = tx.try_reserve().expect("reserved 2");
        assert!(matches!(tx.try_reserve(), Err(TryReserveError::Full)));
        assert!(matches!(tx.try_reserve(), Err(TryReserveError::Full)));
        drop(rx);
        assert!(matches!(
            tx.try_reserve(),
            Err(TryReserveError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn recv_future_awoken_but_unused() {
        let (tx, rx) = channel::<i32>(1);
        let mut recv = Box::pin(rx.recv());
        let rx2 = rx.clone();
        // Try receiving from rx2, but don't drop it yet.
        tokio::select! {
            biased;
            _ = &mut recv => {
                panic!("unexpected recv");
            }
            _ = ReadyFuture {} => {}
        }
        let task = tokio::spawn(async move { rx2.recv().await });
        // Yield the current task so task above can be started.
        tokio::time::sleep(Duration::ZERO).await;
        tx.try_send(1).expect("sent");
        // It would hang without the drop, since the recv would be awoken, but we'd never await for
        // it. This is the main flaw of this design where only a single future is awoken at the
        // time. Alternatively, we could wake all of them at once, but this would most likely
        // result in performance degradation due to lock contention.
        drop(recv);
        let res = task.await.expect("no panic").expect("received");
        assert_eq!(res, 1);
    }

    #[tokio::test]
    async fn try_reserve_unused_permit_and_send() {
        let (tx, rx) = channel::<i32>(1);
        let permit = tx.try_reserve().expect("reserved");
        let task = tokio::spawn({
            let tx = tx.clone();
            async move { tx.send(1).await }
        });
        drop(permit);
        task.await.expect("no panic").expect("sent");
        assert_eq!(rx.try_recv().expect("recv"), 1);
    }

    #[tokio::test]
    async fn try_reserve_unused_permit_and_other_permit() {
        let (tx, rx) = channel::<i32>(1);
        let permit = tx.try_reserve().expect("reserved");
        let task = tokio::spawn({
            let tx = tx.clone();
            async move { tx.reserve().await.expect("reserved").send(1) }
        });
        drop(permit);
        task.await.expect("no panic");
        assert_eq!(rx.try_recv().expect("recv"), 1);
    }

    #[tokio::test]
    async fn receiver_close_all() {
        let (tx, rx1) = channel::<i32>(3);
        let rx2 = rx1.clone();
        let permit1 = tx.reserve().await.unwrap();
        let permit2 = tx.reserve().await.unwrap();
        tx.send(1).await.unwrap();
        rx1.close_all();
        assert_no_recv(&rx1).await;
        assert_no_recv(&rx2).await;
        permit1.send(2);
        permit2.send(3);
        assert_eq!(rx1.recv().await.unwrap(), 2);
        assert_eq!(rx2.try_recv().unwrap(), 3);
        assert_eq!(rx1.recv().await.unwrap(), 1);
        assert_eq!(rx1.recv().await, Err(RecvError));
        assert_eq!(rx2.recv().await, Err(RecvError));
        assert!(matches!(tx.send(3).await, Err(SendError(3))));
    }

    #[tokio::test]
    async fn receiver_close_all_permit_drop() {
        let (tx, rx) = channel::<i32>(3);
        let permit = tx.reserve().await.unwrap();
        rx.close_all();
        assert_no_recv(&rx).await;
        drop(permit);
        assert_eq!(rx.recv().await, Err(RecvError));
    }

    #[tokio::test]
    async fn reserve_owned() {
        let (tx, rx) = channel::<usize>(4);
        let tx = tx.reserve_owned().await.unwrap().send(1);
        let tx = tx.reserve_owned().await.unwrap().send(2);
        let tx = tx.try_reserve_owned().unwrap().send(3);
        let tx = tx.try_reserve_owned().unwrap().release();
        let tx = tx.try_reserve_owned().unwrap().send(4);
        assert!(matches!(
            tx.clone().try_reserve_owned(),
            Err(TrySendError::Full(_))
        ));
        for i in 1..=4 {
            assert_eq!(rx.try_recv().unwrap(), i);
        }
        drop(rx);
        assert!(matches!(
            tx.clone().reserve_owned().await,
            Err(ReserveError)
        ));
        assert!(matches!(
            tx.try_reserve_owned(),
            Err(TrySendError::Disconnected(_))
        ));
    }

    #[tokio::test]
    async fn reserve_many() {
        let (tx, rx) = channel::<usize>(10);
        let p1 = tx.reserve_many(5).await.unwrap();
        let p2 = tx.try_reserve_many(5).unwrap();
        assert!(matches!(tx.try_send(11), Err(TrySendError::Full(11))));
        for (i, p) in p2.enumerate() {
            p.send(i + 5);
        }
        for (i, p) in p1.enumerate() {
            p.send(i);
        }
        for i in 0..10 {
            assert_eq!(rx.try_recv().unwrap(), i);
        }
    }

    #[tokio::test]
    async fn reserve_many_drop() {
        let (tx, _rx) = channel::<usize>(2);
        let it = tx.reserve_many(2).await.unwrap();
        drop(it);
        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        assert!(matches!(tx.try_send(3), Err(TrySendError::Full(3))));
    }

    #[tokio::test]
    async fn reserve_many_drop_halfway() {
        let (tx, _rx) = channel::<usize>(4);
        let mut it = tx.reserve_many(4).await.unwrap();
        it.next().unwrap().send(1);
        it.next().unwrap().send(2);
        drop(it);
        tx.try_send(3).unwrap();
        tx.try_send(4).unwrap();
        assert!(matches!(tx.try_send(5), Err(TrySendError::Full(5))));
    }

    async fn assert_no_recv<T>(rx: &Receiver<T>)
    where
        T: Debug,
    {
        tokio::select! {
            result = rx.recv() => {
                panic!("unexpected recv: {result:?}");
            },
            _ = tokio::time::sleep(std::time::Duration::ZERO) => {},
        }
    }

    struct ReadyFuture {}

    impl Future for ReadyFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(())
        }
    }
}
