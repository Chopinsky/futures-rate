use std::future::Future;
use std::time::Duration;
use std::thread;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use crate::enter;
use crate::semaphore::Semaphore;
use crate::threads_queue::ThreadsQueue;

pub(crate) struct InnerPool(AtomicUsize, ThreadsQueue);

pub struct TokenPool {
    inner: Arc<InnerPool>,
}

impl TokenPool {
    pub fn new(size: usize) -> Self {
        enter::arrive();

        TokenPool {
            inner: Arc::new(InnerPool(
                AtomicUsize::new(size),
                ThreadsQueue::new()
            )),
        }
    }

    pub fn with_semaphore<R, F>(&self, fut: F) -> Semaphore<R, F>
    where
        R: Send + 'static,
        F: Future<Output = R> + 'static,
    {
        Semaphore::new(fut, Arc::clone(&self.inner))
    }
}

impl Drop for TokenPool {
    fn drop(&mut self) {
        enter::depart();
    }
}

pub(crate) trait TokenFetcher {
    fn add_token(&self, count: usize);
    fn wait_for_token(&self);
    fn return_token(&self);
}

impl TokenFetcher for InnerPool {
    fn add_token(&self, count: usize) {
        self.0.fetch_add(count, Ordering::AcqRel);
    }

    fn wait_for_token(&self) {
        let mut curr = 1;
        let mut attempts = 1;

        while let Err(val) =
            self.0.compare_exchange(curr, curr - 1, Ordering::SeqCst, Ordering::Relaxed)
        {
            if val == 0 && !enter::test() {
                // no token available at the moment, decide what to do, put the current thread
                // into the queue
                self.1.enqueue(thread::current());

                // put to sleep for now
                thread::park();

                // now we wake up in wake of a new token available
                return;
            }

            if val == 0 || attempts > 4 {
                // token is available but we can't grab it just yet, or there's no token but we
                // can't park since we're in some pool's main thread --> let's take a break from
                // contentious competitions.
                if attempts < 10 {
                    thread::sleep(Duration::from_micros(attempts * 100));
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
        if let Some(th) = self.1.dequeue() {
            // the front thread take the token, total number is unchanged
            th.unpark();
            return;
        }

        // return the token to the pool
        self.0.fetch_add(1, Ordering::Release);
    }
}
