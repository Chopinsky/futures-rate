use futures::{executor, future};
use futures_rate::{Semaphore, TokenPool};
use std::future::Future;
use std::thread;
use std::time::Duration;

fn main() {
    let token_pool = TokenPool::new(1);

    let fut_values = async {
        let fut_1 = build_fut(0, &token_pool);
        let fut_2 = build_fut(1, &token_pool);

        let fin = future::join(fut_1, fut_2);
        fin.await
    };

    let values = executor::block_on(fut_values);

    println!("Values from fut_1={:?}", values.0);
    println!("Values from fut_2={:?}", values.1);
}

fn build_fut(
    offset: i32,
    token_pool: &TokenPool,
) -> Semaphore<Vec<i32>, impl Future<Output = Vec<i32>>> {
    token_pool.register(async move {
        let mut values = Vec::with_capacity(100);
        (0..100).for_each(|v| {
            thread::sleep(Duration::from_millis(1));
            values.push(2 * v + offset);
        });

        values
    })
}
