use crate::permit::Permit;
use crate::threads_queue::ThreadsQueue;
use crate::{enter, RatioType};
use std::future::Future;
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc, Weak, RwLock};
use std::thread;
use std::time::Duration;
use std::sync::atomic::AtomicBool;

pub(crate) struct InnerPool {
    token_counts: AtomicUsize,
    deficit: AtomicUsize,
    parking_lot: ThreadsQueue,
    flavor: RwLock<RatioType>,
}

pub struct Semaphore {
    inner: Arc<InnerPool>,
    closed: AtomicBool,
}

impl Semaphore {
    pub fn new(size: usize) -> Self {
        enter::arrive();

        Semaphore {
            inner: Arc::new(InnerPool {
                token_counts: AtomicUsize::new(size),
                deficit: AtomicUsize::new(0),
                parking_lot: ThreadsQueue::new(),
                flavor: RwLock::new(RatioType::Static(size)),
            }),
            closed: AtomicBool::from(false),
        }
    }

    pub fn with_rate(count_per_ms: usize) -> Self {
        enter::arrive();

        let inner_pool = Arc::new(InnerPool {
            token_counts: AtomicUsize::new(0),
            deficit: AtomicUsize::new(0),
            parking_lot: ThreadsQueue::new(),
            flavor: RwLock::new(RatioType::Fixed(count_per_ms)),
        });

        Self::spawn_token_generator(Arc::downgrade(&Arc::clone(&inner_pool)));

        Semaphore {
            inner: inner_pool,
            closed: AtomicBool::new(false),
        }
    }

    pub fn set_ratio(&mut self, ratio: RatioType) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }

        let flavor = self.inner.get_flavor();

        match (flavor, ratio) {
            (RatioType::Static(old), RatioType::Static(new)) => {
                if old < new {
                    // need to add tokens
                    self.inner
                        .token_counts
                        .fetch_add(new - old, Ordering::SeqCst);
                } else if old > new {
                    // need to remove tokens
                    self.inner.deficit.fetch_add(old - new, Ordering::SeqCst);
                } else {
                    // the same, no change needs to be made
                      return;
                }
            }
            (RatioType::Static(_), RatioType::Fixed(_)) => {
                Self::spawn_token_generator(
                    Arc::downgrade(&Arc::clone(&self.inner))
                );
            }
            _ => {}
        };

        self.inner.set_flavor(ratio);
    }

    pub fn register<R, F>(&self, fut: F) -> Option<Permit<R, F>>
    where
        R: Send + 'static,
        F: Future<Output = R> + 'static,
    {
        if self.closed.load(Ordering::Acquire) {
            return None;
        }

        Some(Permit::new(fut, Arc::clone(&self.inner)))
    }

    pub fn close(&self) {
        println!("closing...");

        self.closed.store(true, Ordering::SeqCst);

        while let Some(th) = self.inner.parking_lot.dequeue() {
            th.unpark();
        }
    }

    fn spawn_token_generator(pool: Weak<InnerPool>) {
        thread::spawn(move || loop {
            match pool.upgrade() {
                Some(p) => {

                    if let RatioType::Fixed(rate) = p.get_flavor() {
                        // set amount of tokens allowed in this time slab to the pool
                        p.token_counts.store(rate, Ordering::SeqCst);
                    } else {
                        // we've changed the pool flavor, quit now
                        return;
                    }
                }

                // the pool has quit, we shall too.
                None => return,
            }

            thread::sleep(Duration::from_millis(1));
        });
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        enter::depart();
        self.close();
    }
}

impl InnerPool {
    fn get_flavor(&self) -> RatioType {
        *self.flavor.read().expect("Failed to get the semaphore flavor ... ")
    }

    fn set_flavor(&self, flavor: RatioType) {
        let mut f = self.flavor.write().expect("Failed to set the semaphore flavor ... ");
        *f = flavor;
    }
}

pub(crate) trait TokenFetcher {
    fn add_token(&self, count: usize);
    fn wait_for_token(&self);
    fn return_token(&self);
}

impl TokenFetcher for InnerPool {
    fn add_token(&self, count: usize) {
        self.token_counts.fetch_add(count, Ordering::AcqRel);
    }

    fn wait_for_token(&self) {
        let mut curr = 1;
        let mut attempts = 1;

        while let Err(val) =
            self.token_counts
                .compare_exchange(curr, curr - 1, Ordering::SeqCst, Ordering::Relaxed)
        {
            if val == 0 && !enter::test() {
                // no token available at the moment, decide what to do, put the current thread
                // into the queue
                self.parking_lot.enqueue(thread::current());

                // put to sleep for now
                thread::park();

                // now we wake up in wake of a new token available
                return;
            }

            if val == 0 || attempts > 4 {
                // token is available but we can't grab it just yet, or there's no token but we
                // can't park since we're in some pool's main thread --> let's take a break from
                // contentious competitions.
                if attempts < 12 {
                    thread::sleep(Duration::from_micros(attempts * 10));
                } else {
                    thread::yield_now();
                }
            }

            // we can retry again right away, use
            curr = if val > 0 { val } else { 1 };

            // mark the attempts
            attempts += 1;
        }
    }

    fn return_token(&self) {
        if let Some(th) = self.parking_lot.dequeue() {
            // the front thread take the token, total number is unchanged
            th.unpark();
            return;
        }

        if let RatioType::Static(_) = self.get_flavor() {
            let mut deficit = self.deficit.load(Ordering::Acquire);

            if deficit == 0 {
                self.token_counts.fetch_add(1, Ordering::AcqRel);
            } else {
                let mut attempts = 1;

                while let Err(curr) = self.deficit.compare_exchange(
                    deficit,
                    deficit - 1,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    if curr == 0 {
                        self.token_counts.fetch_add(1, Ordering::AcqRel);
                        return;
                    }

                    deficit = curr;

                    thread::sleep(Duration::from_micros(1 << attempts));

                    if attempts < 8 {
                        attempts += 1;
                    }
                }
            }
        }
    }
}
