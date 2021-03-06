#![allow(deprecated)]

use crate::controller::Controller;
use crate::inner::{InnerPool, TokenFetcher};
use crate::pass::{Envelope, Permit, Ticket};
use crate::{enter, InterruptedReason, RatioType, TokenPolicy, SpinPolicy};
use std::future::Future;
use std::sync::{Arc, Weak};
use std::thread;
use std::time::Duration;

struct KeeperPolicy {
    token: TokenPolicy,
    spin: SpinPolicy,
}

impl Default for KeeperPolicy {
    fn default() -> Self {
        KeeperPolicy {
            token: TokenPolicy::Preemptive,
            spin: SpinPolicy::InplaceWait,
        }
    }
}

pub struct GateKeeper {
    inner: Arc<InnerPool>,
    policy: KeeperPolicy,
}

impl GateKeeper {
    pub fn new(size: usize) -> Self {
        assert!(size > 0);

        enter::arrive();

        GateKeeper {
            inner: Arc::new(InnerPool::new(size, RatioType::Static(size))),
            policy: Default::default(),
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
            policy: Default::default(),
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

        let mut fut_wrapper = Some(fut);

        let mut ticket: Ticket<R, F> = match self.policy.token {
            TokenPolicy::Cooperative => Ticket::new(Arc::clone(&self.inner), fut_wrapper.take()),
            TokenPolicy::Preemptive => Ticket::new(Arc::clone(&self.inner), None),
        };

        if self.policy.spin != SpinPolicy::InplaceWait {
            ticket.set_spin_policy(self.policy.spin);
        }

        let fut = async move {
            let _stub = match ticket.await {
                Ok(Envelope::Stub(t)) => t,
                Ok(Envelope::Output(val)) => return val,
                Err(_e) => panic!("issuance of the pass has been disrupted unexpectedly ..."),
            };

            fut_wrapper.take().unwrap().await
        };

        Some(fut)
    }

    pub fn issue_interruptable<R, F>(
        &self,
        fut: F,
    ) -> Option<(impl Future<Output = Result<R, InterruptedReason>>, Controller)>
    where
        R: Send + 'static,
        F: Future<Output = R> + 'static,
    {
        if self.is_closed() {
            return None;
        }

        let ticket: Ticket<R, F> = Ticket::new(Arc::clone(&self.inner), None);

        Some((
            async move {
                ticket.await?;
                Ok(fut.await)
            },
            Controller {},
         ))
    }

    pub fn close(&self) {
        self.inner.close();
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    fn spawn_token_generator(pool: Weak<InnerPool>) {
        thread::spawn(move || {
            loop {
                if let Some(p) = pool.upgrade() {
                    // if the pool ref still exists
                    if let RatioType::FixedRate(count, d) = p.get_flavor() {
                        // set amount of tokens allowed in this time slab to the pool
                        p.reset_token(count);
                        thread::sleep(d);

                        continue;
                    }
                }

                // the pool has quit, we shall too.
                return;
            }
        });
    }
}

pub trait GateKeeperConfig {
    fn set_ratio(&mut self, ratio: RatioType);
    fn set_token_policy(&mut self, policy: TokenPolicy);
    fn set_spin_policy(&mut self, spin: SpinPolicy);
}

impl GateKeeperConfig for GateKeeper {
    fn set_token_policy(&mut self, policy: TokenPolicy) {
        self.policy.token = policy;
    }

    fn set_spin_policy(&mut self, spin: SpinPolicy) {
        self.policy.spin = spin;
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
