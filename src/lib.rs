use std::collections::{BTreeMap, VecDeque};
use std::fmt::Display;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use std::future::Future;

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

/// Creates a new bounded channel.
pub fn channel<T>(cap: usize) -> (Sender<T>, Receiver<T>)
where
    T: Send + Sync + 'static,
{
    let state = State {
        inner: Arc::new(Mutex::new(InnerState {
            buffer: VecDeque::with_capacity(cap),
            reserved_count: 0,
            receivers_count: 0,
            senders_count: 0,
            waiting_send_futures: BTreeMap::new(),
            waiting_recv_futures: BTreeMap::new(),
            next_waker_id: 0,
            disconnected: false,
        })),
    };

    (state.new_sender(), state.new_receiver())
}

pub struct Receiver<T>
where
    T: Send + Sync + 'static,
{
    state: State<T>,
}

impl<T> Clone for Receiver<T>
where
    T: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        self.state.new_receiver()
    }
}

impl<T> Receiver<T>
where
    T: Send + Sync + 'static,
{
    fn new(state: State<T>) -> Self {
        Self { state }
    }

    pub async fn recv(&self) -> Result<T, RecvError> {
        let recv = RecvFuture::new(self.state.next_waker_id(), self.state.cloned());

        recv.await
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let mut state = self.state.inner_mut();

        if let Some(value) = state.buffer.pop_front() {
            Ok(value)
        } else if state.disconnected {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }
}

impl<T> Drop for Receiver<T>
where
    T: Send + Sync + 'static,
{
    fn drop(&mut self) {
        self.state.drop_receiver();
    }
}

pub struct Sender<T>
where
    T: Send + Sync + 'static,
{
    state: State<T>,
}

impl<T> Clone for Sender<T>
where
    T: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        self.state.new_sender()
    }
}

impl<T> Sender<T>
where
    T: Send + Sync + 'static,
{
    fn new(state: State<T>) -> Sender<T> {
        Self { state }
    }

    /// Waits until the value is sent or returns an when all receivers have been dropped.
    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
        let send = SendFuture::new(self.state.next_waker_id(), self.state.cloned(), value);

        send.await
    }

    /// Sends without blocking or returns an when all receivers have been dropped.
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        let mut state = self.state.inner_mut();

        if state.disconnected {
            return Err(TrySendError::Disconnected(value));
        }

        if state.has_room_for(1) {
            if let Some(waker) = state.send_and_get_waker(value) {
                drop(state);
                waker.wake();
            }
            Ok(())
        } else {
            Err(TrySendError::Full(value))
        }
    }

    /// Waits until a permit is reserved or returns an when all receivers have been dropped.
    pub async fn reserve(&self) -> Result<Permit<T>, ReserveError> {
        let reserve = ReserveFuture::new(self.state.next_waker_id(), self.state.cloned());

        reserve.await
    }

    /// Reserves a permit or returns an when all receivers have been dropped.
    pub fn try_reserve(&self) -> Result<Permit<T>, TryReserveError> {
        let mut state = self.state.inner_mut();

        if state.disconnected {
            return Err(TryReserveError::Disconnected);
        }

        if state.has_room_for(1) {
            state.reserved_count += 1;
            Ok(Permit::new(self.state.cloned()))
        } else {
            Err(TryReserveError::Full)
        }
    }
}

impl<T> Drop for Sender<T>
where
    T: Send + Sync + 'static,
{
    fn drop(&mut self) {
        self.state.drop_sender();
    }
}

#[derive(Debug, Clone)]
struct State<T> {
    inner: Arc<Mutex<InnerState<T>>>,
}

impl<T> State<T>
where
    T: Send + Sync + 'static,
{
    fn cloned(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }

    fn inner_mut(&self) -> impl DerefMut<Target = InnerState<T>> + '_ {
        self.inner.lock().unwrap()
    }

    fn next_waker_id(&self) -> WakerId {
        self.inner_mut().next_waker_id()
    }

    fn new_sender(&self) -> Sender<T> {
        self.inner_mut().senders_count += 1;
        Sender::new(self.cloned())
    }

    fn new_receiver(&self) -> Receiver<T> {
        self.inner_mut().receivers_count += 1;
        Receiver::new(self.cloned())
    }

    fn drop_sender(&self) {
        let wakers = {
            let mut inner = self.inner_mut();
            inner.senders_count -= 1;

            if inner.senders_count == 0 {
                Some(inner.mark_disconnected_and_take_recv_futures())
            } else {
                None
            }
        };

        if let Some(wakers) = wakers {
            for waker in wakers {
                waker.wake()
            }
        }
    }

    fn drop_receiver(&self) {
        let wakers = {
            let mut inner = self.inner_mut();
            inner.receivers_count -= 1;

            if inner.receivers_count == 0 {
                Some(inner.mark_disconnected_and_take_send_futures())
            } else {
                None
            }
        };

        if let Some(wakers) = wakers {
            for waker in wakers {
                waker.wake()
            }
        }
    }

    fn drop_permit(&self, has_sent: bool) {
        let waker = {
            let mut inner = self.inner_mut();
            inner.reserved_count -= 1;

            // When the permit was not used for sending, it means a spot was freed, so we can notify
            // one of the senders to proceed.
            if !has_sent {
                inner.waiting_send_futures.pop_first()
            } else {
                None
            }
        };

        if let Some((_, waker)) = waker {
            waker.wake();
        }
    }

    fn drop_send_future(&self, id: &WakerId, has_sent: bool) {
        let waker = {
            let mut inner = self.inner_mut();

            let was_awoken = inner.waiting_send_futures.remove(id).is_some();

            // Wake another sender in case this one has been awoken, but it was dropped before it
            // managed to send anything.
            if was_awoken && !has_sent {
                inner.take_one_send_future_waker()
            } else {
                None
            }
        };

        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn drop_recv_future(&self, id: &WakerId, has_received: bool) {
        let waker = {
            let mut inner = self.inner_mut();
            let was_awoken = inner.waiting_recv_futures.remove(id).is_none();

            // Wake another receiver in case this one has been awoken, but it was dropped before it
            // managed to receive anything.
            if was_awoken {
                if has_received {
                    // If we have received, it means a spot was freed in the internal buffer, so wake
                    // one sender.
                    inner.take_one_send_future_waker()
                } else {
                    // If we have not received, it means another RecvFuture might take over.
                    inner.take_one_recv_future_waker()
                }
            } else {
                None
            }
        };

        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

#[derive(Debug)]
struct InnerState<T> {
    /// Buffered messages, capacity is fixed.
    buffer: VecDeque<T>,
    /// Number of reserved spots for sending.
    reserved_count: usize,
    /// Total of created receivers.
    receivers_count: usize,
    /// Total of created senders.
    senders_count: usize,
    /// Senders waiting to send.
    waiting_send_futures: BTreeMap<WakerId, Waker>,
    /// Receivers waiting to receive.
    waiting_recv_futures: BTreeMap<WakerId, Waker>,
    /// Unique waker ID so each future can remove its own waker on Drop.
    next_waker_id: WakerId,
    /// False by default, true when all senders or all receivers are dropped.
    disconnected: bool,
}

impl<T> InnerState<T>
where
    T: Send + Sync + 'static,
{
    fn next_waker_id(&mut self) -> WakerId {
        let waker_id = self.next_waker_id;
        self.next_waker_id += 1;
        waker_id
    }

    fn has_room_for(&self, required_num_items: usize) -> bool {
        let space = self.buffer.capacity() - self.buffer.len() - self.reserved_count;
        space >= required_num_items
    }

    #[must_use]
    fn send_and_get_waker(&mut self, value: T) -> Option<Waker> {
        self.buffer.push_back(value);
        self.take_one_recv_future_waker()
    }

    #[must_use]
    fn mark_disconnected_and_take_recv_futures(&mut self) -> impl Iterator<Item = Waker> {
        self.disconnected = true;
        Self::take_all_wakers(&mut self.waiting_recv_futures).into_values()
    }

    #[must_use]
    fn mark_disconnected_and_take_send_futures(&mut self) -> impl Iterator<Item = Waker> {
        self.disconnected = true;
        Self::take_all_wakers(&mut self.waiting_send_futures).into_values()
    }

    #[must_use]
    fn take_one_send_future_waker(&mut self) -> Option<Waker> {
        Self::take_one_waker(&mut self.waiting_send_futures)
    }

    #[must_use]
    fn take_one_recv_future_waker(&mut self) -> Option<Waker> {
        Self::take_one_waker(&mut self.waiting_recv_futures)
    }

    #[must_use]
    fn take_one_waker(map: &mut BTreeMap<WakerId, Waker>) -> Option<Waker> {
        map.pop_first().map(|(_, waker)| waker)
    }

    fn take_all_wakers(map: &mut BTreeMap<WakerId, Waker>) -> BTreeMap<WakerId, Waker> {
        std::mem::take(map)
    }
}

type WakerId = usize;

struct SendFuture<T>
where
    T: Send + Sync + 'static,
{
    id: WakerId,
    state: State<T>,
    value: Option<T>,
}

impl<T> SendFuture<T>
where
    T: Send + Sync + 'static,
{
    fn new(id: WakerId, state: State<T>, value: T) -> Self {
        Self {
            id,
            state,
            value: Some(value),
        }
    }
}

impl<T> Unpin for SendFuture<T> where T: Send + Sync + 'static {}

impl<T> Future for SendFuture<T>
where
    T: Send + Sync + 'static,
{
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Already sent this value.
        if self.value.is_none() {
            return Poll::Ready(Ok(()));
        }

        let this = self.deref_mut();
        let mut state = this.state.inner_mut();

        if state.disconnected {
            Poll::Ready(Err(SendError(this.value.take().expect("some value"))))
        } else if state.has_room_for(1) {
            if let Some(waker) = state.send_and_get_waker(this.value.take().expect("some value")) {
                drop(state);
                waker.wake();
            }
            Poll::Ready(Ok(()))
        } else {
            state
                .waiting_send_futures
                .insert(this.id, cx.waker().clone());
            Poll::Pending
        }
    }
}

impl<T> Drop for SendFuture<T>
where
    T: Send + Sync + 'static,
{
    fn drop(&mut self) {
        let has_sent = self.value.is_none();
        self.state.drop_send_future(&self.id, has_sent);
    }
}

#[derive(Debug)]
pub struct Permit<T>
where
    T: Send + Sync + 'static,
{
    state: State<T>,
    has_sent: bool,
}

impl<T> Permit<T>
where
    T: Send + Sync + 'static,
{
    fn new(state: State<T>) -> Self {
        Self {
            state,
            has_sent: false,
        }
    }

    pub fn send(mut self, value: T) {
        if let Some(waker) = self.state.inner_mut().send_and_get_waker(value) {
            waker.wake();
        }
        self.has_sent = true;
        drop(self)
    }
}

impl<T> Drop for Permit<T>
where
    T: Send + Sync + 'static,
{
    fn drop(&mut self) {
        self.state.drop_permit(self.has_sent);
    }
}

struct ReserveFuture<T>
where
    T: Send + Sync + 'static,
{
    id: WakerId,
    state: State<T>,
    has_reserved: bool,
}

impl<T> ReserveFuture<T>
where
    T: Send + Sync + 'static,
{
    fn new(id: WakerId, state: State<T>) -> Self {
        Self {
            id,
            state,
            has_reserved: false,
        }
    }
}

impl<T> Unpin for ReserveFuture<T> where T: Send + Sync + 'static {}

impl<T> Future for ReserveFuture<T>
where
    T: Send + Sync + 'static,
{
    type Output = Result<Permit<T>, ReserveError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.has_reserved {
            panic!("ReservedFuture polled after ready");
        }

        let this = self.deref_mut();
        let mut state = this.state.inner_mut();
        if state.disconnected {
            Poll::Ready(Err(ReserveError))
        } else if state.has_room_for(1) {
            this.has_reserved = true;
            state.reserved_count += 1;
            drop(state);
            let reserved = Permit::new(self.state.cloned());
            Poll::Ready(Ok(reserved))
        } else {
            state
                .waiting_send_futures
                .insert(this.id, cx.waker().clone());
            Poll::Pending
        }
    }
}

struct RecvFuture<T>
where
    T: Send + Sync + 'static,
{
    id: WakerId,
    state: State<T>,
    has_received: bool,
}

impl<T> RecvFuture<T>
where
    T: Send + Sync + 'static,
{
    fn new(id: WakerId, state: State<T>) -> Self {
        Self {
            id,
            state,
            has_received: false,
        }
    }
}

impl<T> Unpin for RecvFuture<T> where T: Send + Sync + 'static {}

impl<T> Future for RecvFuture<T>
where
    T: Send + Sync + 'static,
{
    type Output = Result<T, RecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.deref_mut();
        let mut state = this.state.inner_mut();
        let value = state.buffer.pop_front();

        match value {
            Some(value) => {
                this.has_received = true;
                Poll::Ready(Ok(value))
            }
            None => {
                if state.disconnected {
                    Poll::Ready(Err(RecvError))
                } else {
                    state
                        .waiting_recv_futures
                        .insert(this.id, cx.waker().clone());
                    if let Some(waker) = state.take_one_send_future_waker() {
                        drop(state);
                        waker.wake();
                    }
                    Poll::Pending
                }
            }
        }
    }
}

impl<T> Drop for RecvFuture<T>
where
    T: Send + Sync + 'static,
{
    fn drop(&mut self) {
        self.state.drop_recv_future(&self.id, self.has_received);
    }
}

#[cfg(test)]
mod testing {
    use std::collections::BTreeSet;
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
                        return tx;
                    }
                    Err(_err) => {
                        tokio::time::sleep(Duration::ZERO).await;
                    }
                }
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
            async move { tx.reserve().await }
        });
        drop(rx);
        assert!(matches!(tx.reserve().await, Err(ReserveError)));
        assert!(matches!(task.await.expect("panic"), Err(ReserveError)));
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
        let res = task.await.expect("no panic").expect("receivd");
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

    struct ReadyFuture {}

    impl Future for ReadyFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(())
        }
    }
}
