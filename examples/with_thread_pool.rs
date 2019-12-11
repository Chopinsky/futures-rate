use futures::channel::mpsc::{self, UnboundedSender};
use futures::executor::ThreadPool;
use futures::StreamExt;
use futures::{executor, Future};
use futures_rate::{GateKeeper, Permit};
use std::thread;
use std::time::Duration;

fn main() {
    let pool = ThreadPool::new().expect("Failed to build pool");
    let (tx, rx) = mpsc::unbounded::<i32>();
    let gatekeeper = GateKeeper::new(1);

    let fut_values = async {
        let fut_1 = build_fut(&tx, &gatekeeper);

        pool.spawn_ok(fut_1);

        let fut_2 = build_fut(&tx, &gatekeeper);

        pool.spawn_ok(fut_2);

        drop(tx);

        let fut_values = rx.map(|v| v * 2).collect();

        fut_values.await
    };

    let values: Vec<i32> = executor::block_on(fut_values);

    println!("Values={:?}", values);
}

fn build_fut(
    tx: &UnboundedSender<i32>,
    gatekeeper: &GateKeeper,
) -> Permit<(), impl Future<Output = ()>> {
    let tx_clone = tx.clone();

    gatekeeper
        .register(async move {
            (0..100).for_each(|v| {
                thread::sleep(Duration::from_millis(1));
                tx_clone.unbounded_send(v).expect("Failed to send");
            });
        })
        .unwrap()
}
