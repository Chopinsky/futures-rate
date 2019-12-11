use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use futures::{executor, future};
use futures_rate::{GateKeeper, Permit};

static SLOT: AtomicBool = AtomicBool::new(false);

fn main() {
    let gatekeeper = GateKeeper::new(1);

    let fut_values = async {
        let fut_1 = build_fut(&gatekeeper);
        let fut_2 = build_fut(&gatekeeper);
        let fut_3 = build_fut(&gatekeeper);

        let fin = future::join3(fut_1, fut_2, fut_3);
        fin.await
    };

    executor::block_on(fut_values);

    println!("Values in SLOT = {:?}", SLOT.load(Ordering::SeqCst));
}

fn build_fut(gatekeeper: &GateKeeper) -> Permit<(), impl Future<Output = ()>> {
    gatekeeper
        .register(async move {
            // only 1 future can access the resource at any given time
            assert!(SLOT.compare_exchange(
                false,
                true,
                Ordering::Acquire,
                Ordering::Acquire
            ).is_ok());

            // reset to default value
            SLOT.store(false, Ordering::Release);
        })
        .unwrap()
}
