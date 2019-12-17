use crate::threads_queue::WaitingList;
use crate::RatioType;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::Waker;
use std::thread;
use std::time::Duration;
use std::sync::RwLock;

static UID: AtomicUsize = AtomicUsize::new(1);

pub(crate) struct InnerPool {
    pool_id: usize,
    closed: AtomicBool,
    token_counts: AtomicUsize,
    deficit: AtomicUsize,
    //    parking_lot: ThreadsQueue,
    waiting_list: WaitingList,
    flavor: (RwLock<RatioType>, AtomicBool),
}

impl InnerPool {
    pub(crate) fn new(size: usize, flavor: RatioType) -> Self {
        let is_static_ratio = flavor.is_static_ratio();

        InnerPool {
            pool_id: UID.fetch_add(1, Ordering::SeqCst),
            closed: AtomicBool::from(false),
            token_counts: AtomicUsize::new(size),
            deficit: AtomicUsize::new(0),
            waiting_list: WaitingList::new(),
            flavor: (RwLock::new(flavor), AtomicBool::new(is_static_ratio)),
        }
    }

    #[inline]
    pub(crate) fn get_id(&self) -> usize {
        self.pool_id
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);

        while let Some(waker) = self.waiting_list.dequeue() {
            waker.wake();
        }
    }

    #[inline]
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn get_flavor(&self) -> RatioType {
        *self.flavor.0.read().expect("the rate limit controller is corrupted ...")
    }

    pub(crate) fn set_flavor(&self, flavor: RatioType) {
        self.flavor.1.store(flavor.is_static_ratio(), Ordering::Release);
        *self.flavor.0.write().expect("the rate limit controller is corrupted ...") = flavor;
    }

    pub(crate) fn set_tokens(&self, count: usize) {
        self.token_counts.store(count, Ordering::SeqCst);

        if count > 0 {
            self.wake_up_many(count);
        }
    }

    pub(crate) fn static_rebalance(&self, from: usize, to: usize) {
        if from < to {
            // need to add tokens
            self.token_counts.fetch_add(to - from, Ordering::SeqCst);
        } else if from > to {
            // need to remove tokens
            self.deficit.fetch_add(from - to, Ordering::SeqCst);
        }
    }

    fn recover_one(&self) {
        // return the token back
        self.token_counts.fetch_add(1, Ordering::AcqRel);

        // if we have tickets waiting, wake them up and let them poll again.
        if let Some(waker) = self.waiting_list.dequeue() {
            waker.wake();
        }
    }

    fn wake_up_many(&self, count: usize) {
        assert!(count > 0);

        let mut remainder = count;
        while let Some(waker) = self.waiting_list.dequeue() {
            waker.wake();

            remainder -= 1;
            if remainder == 0 {
                return;
            }
        }
    }
}

pub(crate) trait TokenFetcher {
    fn add_token(&self, count: usize);
    fn request_token(&self, fill_or_cancel: bool) -> bool;
    fn return_token(&self);
    fn enqueue(&self, waker: Waker);
}

impl TokenFetcher for InnerPool {
    fn add_token(&self, count: usize) {
        if count == 0 {
            return;
        }

        self.token_counts.fetch_add(count, Ordering::AcqRel);
        self.wake_up_many(count);
    }

    fn request_token(&self, immediate_or_cancel: bool) -> bool {
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

        // if not a static ratio flavor, we won't return the token back.
        if !self.flavor.1.load(Ordering::Acquire) {
            return;
        }

        // if a static ratio flavor, check if we're in debt first.
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

    fn enqueue(&self, waker: Waker) {
        self.waiting_list.enqueue(waker);
    }
}
