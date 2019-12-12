#![allow(deprecated)]

use crate::pass::{Permit, Ticket};
use crate::threads_queue::WaitingList;
use crate::{enter, RatioType};
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, RwLock, Weak,
};
use std::task::Waker;
use std::thread;
use std::time::Duration;

static UID: AtomicUsize = AtomicUsize::new(1);

pub(crate) struct InnerPool {
    pool_id: usize,
    closed: AtomicBool,
    token_counts: AtomicUsize,
    deficit: AtomicUsize,
    //    parking_lot: ThreadsQueue,
    waiting_list: WaitingList,
    flavor: RwLock<RatioType>,
}

impl InnerPool {
    #[inline]
    pub(crate) fn get_id(&self) -> usize {
        self.pool_id
    }

    #[inline]
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn get_flavor(&self) -> RatioType {
        *self
            .flavor
            .read()
            .expect("Failed to get the GateKeeper flavor ... ")
    }

    fn set_flavor(&self, flavor: RatioType) {
        let mut f = self
            .flavor
            .write()
            .expect("Failed to set the GateKeeper flavor ... ");

        *f = flavor;
    }

    fn recover_one(&self) {
        // return the token back
        self.token_counts.fetch_add(1, Ordering::AcqRel);

        // if we have tickets waiting, wake them up and let them poll again.
        if let Some(waker) = self.waiting_list.dequeue() {
            waker.wake();
        }
    }
}

pub(crate) trait TokenFetcher {
    fn add_token(&self, count: usize);
    fn wait_for_token(&self, fill_or_cancel: bool) -> bool;
    fn return_token(&self);
    fn enqueue(&self, waker: Waker);
}

impl TokenFetcher for InnerPool {
    fn add_token(&self, count: usize) {
        self.token_counts.fetch_add(count, Ordering::AcqRel);
    }

    fn wait_for_token(&self, immediate_or_cancel: bool) -> bool {
        let mut curr = 1;
        let mut attempts = 0;

        while let Err(val) =
            self.token_counts
                .compare_exchange(curr, curr - 1, Ordering::SeqCst, Ordering::Relaxed)
        {
            if val == 0 && immediate_or_cancel {
                return false;
            }

            /*
                        if val == 0 && !enter::test() {
                            // no token available at the moment, decide what to do, put the current thread
                            // into the queue
                            self.parking_lot.enqueue(thread::current());

                            // put to sleep for now
                            thread::park();

                            // now we wake up because a new token available
                            return true;
                        }
            */

            // mark the attempts
            attempts += 1;

            // token is available but we can't grab it just yet, or there's no token but we
            // can't park since we're in some pool's main thread --> let's take a break from
            // contentious competitions.
            if attempts < 8 {
                thread::sleep(Duration::from_micros(1 << attempts));
            } else {
                attempts = 1;
                thread::yield_now();
            }

            // we can retry again right away, use
            curr = if val > 0 { val } else { 1 };
        }

        true
    }

    fn return_token(&self) {
        /*
                if let Some(th) = self.parking_lot.dequeue() {
                    // the front thread take the token, total number is unchanged
                    th.unpark();
                    return;
                }
        */

        if let RatioType::Static(_) = self.get_flavor() {
            let mut deficit = self.deficit.load(Ordering::Acquire);

            if deficit == 0 {
                self.recover_one();
                return;
            }

            // be a Lannister and always pay your debt first
            let mut attempts = 1;

            while let Err(curr) = self.deficit.compare_exchange(
                deficit,
                deficit - 1,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                if curr == 0 {
                    self.recover_one();
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

    fn enqueue(&self, waker: Waker) {
        self.waiting_list.enqueue(waker);
    }
}

pub struct GateKeeper {
    inner: Arc<InnerPool>,
}

impl GateKeeper {
    pub fn new(size: usize) -> Self {
        assert!(size > 0);

        enter::arrive();

        GateKeeper {
            inner: Arc::new(InnerPool {
                pool_id: UID.fetch_add(1, Ordering::SeqCst),
                closed: AtomicBool::from(false),
                token_counts: AtomicUsize::new(size),
                deficit: AtomicUsize::new(0),
                //                parking_lot: ThreadsQueue::new(),
                waiting_list: WaitingList::new(),
                flavor: RwLock::new(RatioType::Static(size)),
            }),
        }
    }

    pub fn with_rate(count_per_ms: usize) -> Self {
        assert!(count_per_ms > 0);

        enter::arrive();

        let inner_pool = Arc::new(InnerPool {
            pool_id: UID.fetch_add(1, Ordering::SeqCst),
            closed: AtomicBool::from(false),
            token_counts: AtomicUsize::new(0),
            deficit: AtomicUsize::new(0),
            //            parking_lot: ThreadsQueue::new(),
            waiting_list: WaitingList::new(),
            flavor: RwLock::new(RatioType::Fixed(count_per_ms)),
        });

        Self::spawn_token_generator(Arc::downgrade(&Arc::clone(&inner_pool)));

        GateKeeper { inner: inner_pool }
    }

    pub fn set_ratio(&mut self, ratio: RatioType) {
        if self.inner.is_closed() {
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
                Self::spawn_token_generator(Arc::downgrade(&Arc::clone(&self.inner)));
            }
            _ => {}
        };

        self.inner.set_flavor(ratio);
    }

    #[deprecated(
        since = "0.1.4",
        note = "\
            This API is deprecated in favor of the `issue()` method, which avoids the deadlock \
            issues as a whole. This API, along with the associated struct `Permit`, will be removed \
            in the 0.1.5 release.
        "
    )]
    pub fn register<R, F>(&self, fut: F) -> Option<Permit<R, F>>
    where
        R: Send + 'static,
        F: Future<Output = R> + 'static,
    {
        if self.is_closed() {
            return None;
        }

        Some(Permit::new(fut, Arc::clone(&self.inner)))
    }

    pub fn issue<R, F>(&self, fut: F) -> Option<impl Future<Output = R>>
    where
        R: Send + 'static,
        F: Future<Output = R> + 'static,
    {
        if self.is_closed() {
            return None;
        }

        let ticket = Ticket::new(Arc::clone(&self.inner));

        Some(async move {
            ticket.await;
            fut.await
        })
    }

    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);

        while let Some(waker) = self.inner.waiting_list.dequeue() {
            waker.wake();
        }

        /*
        while let Some(th) = self.inner.parking_lot.dequeue() {
            th.unpark();
        }
        */
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
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

impl Drop for GateKeeper {
    fn drop(&mut self) {
        enter::depart();
        self.close();
    }
}
