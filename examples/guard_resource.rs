use futures::channel::mpsc::{self, Sender};
use futures::executor::ThreadPool;
use futures::StreamExt;
use futures::{executor, Future};
use futures_rate::GateKeeper;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static SLOT: AtomicBool = AtomicBool::new(false);
static CURR_ACCESS_COUNT: AtomicUsize = AtomicUsize::new(0);

fn main() {
    run_with_keeper();
    run_without_keeper();
}

fn run_with_keeper() {
    let count = 8;

    let pool = ThreadPool::new().expect("Failed to build pool");
    let (tx, rx) = mpsc::channel(count);
    let gatekeeper = GateKeeper::new(1);

    let fut_values = async {
        (0..count).for_each(|_| {
            let fut = build_fut(&tx, &gatekeeper);
            pool.spawn_ok(fut);
        });

        drop(tx);

        rx.map(|_| ()).collect::<Vec<()>>().await
    };

    let count = executor::block_on(fut_values).len();

    println!(
        "After executing {} futures with the futures pool, # of concurrent access to SLOT = {:?}",
        count,
        CURR_ACCESS_COUNT.load(Ordering::SeqCst)
    );
}

fn build_fut(tx: &Sender<()>, gatekeeper: &GateKeeper) -> impl Future<Output = ()> {
    let mut tx_clone = tx.clone();

    gatekeeper
        .issue(async move {
            // only 1 future can access the resource at any given time
            assert!(SLOT
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Acquire)
                .is_ok());

            // reset to default value
            SLOT.store(false, Ordering::Release);

            tx_clone.try_send(()).expect("channel was closed ... ");

            println!("one done ... ");
        })
        .unwrap()
}

fn run_without_keeper() {}
