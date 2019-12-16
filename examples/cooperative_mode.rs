use futures::channel::mpsc::{self, UnboundedSender};
use futures::executor::ThreadPool;
use futures::StreamExt;
use futures::{executor, Future};
use futures_rate::{GateKeeper, GateKeeperConfig, TokenPolicy};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

static OWNER_ID: AtomicUsize = AtomicUsize::new(0);

fn main() {
    let pool = ThreadPool::new().expect("Failed to build pool");
    let (tx, rx) = mpsc::unbounded::<usize>();
    let fut_count = 16;

    let mut keeper = GateKeeper::new(1);
    //    keeper.set_policy(TokenPolicy::Cooperative);

    let fut_main = async {
        (0..fut_count).for_each(|id| {
            let mut path = PathBuf::new();
            path.push("./README.md");

            let fut = keeper.issue(file_reader_fut(id + 1, path, &tx)).unwrap();

            pool.spawn_ok(fut);
        });

        drop(tx);

        rx.map(|count| count).collect().await
    };

    let res: Vec<usize> = executor::block_on(fut_main);
    println!("total line count: {:?}", res);
}

fn file_reader_fut(
    id: usize,
    path: PathBuf,
    tx: &UnboundedSender<usize>,
) -> impl Future<Output = ()> + 'static {
    let tx_clone = tx.clone();

    async move {
        assert!(
            OWNER_ID
                .compare_exchange(0, id, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            format!(
                "{}'s exclusive access is violated ... which should not happen to this test ...",
                id
            )
        );

        // read a file from the location
        let input = match File::open(path) {
            Ok(file) => file,
            Err(_) => return,
        };

        // count number of characters in the file.
        let mut char_count = 0;
        for line in BufReader::new(input).lines() {
            match line {
                Ok(val) => char_count += val.len(),
                Err(_) => return,
            }
        }

        // reset the owner to null
        OWNER_ID.store(0, Ordering::SeqCst);

        // since we own the access, take a nap and see if anyone can invade our space ...
        thread::sleep(Duration::from_micros(4));

        // send the result back...
        tx_clone.unbounded_send(char_count).expect("Failed to send");
    }
}
