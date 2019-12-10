use std::future::Future;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;
use futures::{future, executor};
use bottleneck::{self, semaphore::Semaphore, token_pool::TokenPool};

fn main() {
    let (tx, rx) = mpsc::channel::<i32>();

    let token_pool = TokenPool::new(1);

    let fut_values = async {
        let fut_1 = build_fut(&tx, &token_pool);
        let fut_2 = build_fut(&tx, &token_pool);

        drop(tx);

        let fin = future::join(fut_1, fut_2);
        fin.await
    };

    executor::block_on(fut_values);

    let mut values = Vec::with_capacity(200);
    while let Ok(val) = rx.recv() {
        values.push(val);
    }

    println!("Values={:?}", values);
}

fn build_fut(tx: &Sender<i32>, token_pool: &TokenPool) -> Semaphore<(), impl Future<Output = ()>> {
    let tx_clone = tx.clone();

    token_pool.with_semaphore(async move {
        (0..100).for_each(|v| {
            thread::sleep(Duration::from_millis(1));
            tx_clone.send(v).expect("Failed to send");
        });


    })
}
