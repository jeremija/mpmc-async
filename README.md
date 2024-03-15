# mpmc-async

A multi-producer, multi-consumer async channel with reservations for Rust.

Example usage:

```rust
let (tx, rx) = mpmc_async::channel(1);

let num_workers = 10;
let count = 10;
let mut tasks = Vec::with_capacity(num_workers);

for i in 0..num_workers {
    let mut tx = tx.clone();
    let task = tokio::spawn(async move {
        for j in 0..count {
            let val = i * count + j;
            tx.reserve().await.expect("no error").send(val));
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
```

# LICENSE

[MIT](LICENSE).
