#![allow(deprecated)]

use crate::inner::InnerPool;
use crate::pass::{Permit, Ticket};
use crate::{enter, InterruptedReason, RatioType, TokenPolicy};
use std::future::Future;
use std::sync::{Arc, Weak};
use std::thread;
use std::time::Duration;

pub struct GateKeeper {
    inner: Arc<InnerPool>,
    policy: TokenPolicy,
}

impl GateKeeper {
    pub fn new(size: usize) -> Self {
        assert!(size > 0);

        enter::arrive();

        GateKeeper {
            inner: Arc::new(InnerPool::new(size, RatioType::Static(size))),
            policy: TokenPolicy::Preemptive,
        }
    }

    pub fn with_rate(count_per_interval: usize, interval: Duration) -> Self {
        assert!(count_per_interval > 0);

        enter::arrive();

        let inner_pool = Arc::new(InnerPool::new(
            count_per_interval,
            RatioType::FixedRate(count_per_interval, interval),
        ));

        Self::spawn_token_generator(Arc::downgrade(&Arc::clone(&inner_pool)));

        GateKeeper {
            inner: inner_pool,
            policy: TokenPolicy::Preemptive,
        }
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

        let ticket: Ticket<R, F> = Ticket::new(Arc::clone(&self.inner), None);

        let fut = async move {
            let _stub = match ticket.await {
                Ok((t, _)) => t,
                Err(_e) => panic!("issuance of the pass has been disrupted unexpectedly ..."),
            };

            fut.await
        };

        Some(fut)
    }

    pub fn issue_interruptable<R, F>(
        &self,
        fut: F,
    ) -> Option<impl Future<Output = Result<R, InterruptedReason>>>
    where
        R: Send + 'static,
        F: Future<Output = R> + 'static,
    {
        if self.is_closed() {
            return None;
        }

        let ticket: Ticket<R, F> = Ticket::new(Arc::clone(&self.inner), None);

        Some(
            (
                async move {
                    ticket.await?;
                    Ok(fut.await)
                }
                //            Token {},
            ),
        )
    }

    pub fn close(&self) {
        self.inner.close();

        //        self.inner.for_each(|w| {});
        //        while let Some(waker) = self.inner.waiting_list.dequeue() {
        //            waker.wake();
        //        }

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
        thread::spawn(move || {
            let mut interval;

            loop {
                match pool.upgrade() {
                    // if some pool
                    Some(p) => {
                        if let RatioType::FixedRate(count, d) = p.get_flavor() {
                            // set amount of tokens allowed in this time slab to the pool
                            p.set_tokens(count);
                            interval = d;
                        } else {
                            return;
                        }
                    }

                    // the pool has quit, we shall too.
                    None => return,
                }

                thread::sleep(interval);
            }
        });
    }
}

pub trait GateKeeperConfig {
    fn set_policy(&mut self, policy: TokenPolicy);
    fn set_ratio(&mut self, ratio: RatioType);
}

impl GateKeeperConfig for GateKeeper {
    fn set_policy(&mut self, policy: TokenPolicy) {
        self.policy = policy;
    }

    fn set_ratio(&mut self, ratio: RatioType) {
        if self.inner.is_closed() {
            return;
        }

        let flavor = self.inner.get_flavor();
        self.inner.set_flavor(ratio.clone());

        match (flavor, ratio) {
            (RatioType::Static(from), RatioType::Static(to)) => {
                self.inner.static_rebalance(from, to);
            }
            (RatioType::Static(_), RatioType::FixedRate(_, _)) => {
                Self::spawn_token_generator(Arc::downgrade(&Arc::clone(&self.inner)));
            }
            _ => {}
        };
    }
}

impl Drop for GateKeeper {
    fn drop(&mut self) {
        enter::depart();
        self.close();
    }
}
