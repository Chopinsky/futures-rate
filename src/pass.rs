use crate::gatekeeper::{InnerPool, TokenFetcher};
use std::cell::RefCell;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

thread_local!(
    static TOKEN_BUCKET: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
);

#[deprecated(
    since = "0.1.4",
    note = "\
        The `Permit` is going to be deprecated in favor of calling GateKeeper's `issue()` method \
        to guard the gate. This struct, along with the associated API (i.e. `register()`), will be
        removed in the 0.1.5 release.
    "
)]
pub struct Permit<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    fut: F,
    pool: Arc<InnerPool>,
}

impl<R, F> Permit<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    pub(crate) fn new(fut: F, pool: Arc<InnerPool>) -> Self {
        Permit { fut, pool }
    }
}

impl<R, F> Future for Permit<R, F>
where
    R: Send + 'static,
    F: std::future::Future<Output = R> + 'static,
{
    type Output = R;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        // dissolve the pin to get inner contents
        let (fut, pool) = unsafe {
            let ptr = Pin::get_unchecked_mut(self);

            (Pin::new_unchecked(&mut ptr.fut), &ptr.pool)
        };

        // the current pool's id, to identify the gatekeeper
        let pool_id = pool.get_id();

        // check if the parent future has already obtained the permit
        let need_token = TOKEN_BUCKET.with(|set| !set.borrow().contains(&pool_id));

        // if we're the first future to try the gatekeeper, wait for a permit to be available
        if need_token {
            pool.wait_for_token(false);

            TOKEN_BUCKET.with(|set| {
                (*set.borrow_mut()).insert(pool_id);
            });
        }

        // poll the future, do the actual work
        let res = fut.poll(ctx);

        // now we're ready to return the token, if we're the one requested it
        if need_token {
            TOKEN_BUCKET.with(|set| {
                (*set.borrow_mut()).remove(&pool_id);
            });

            pool.return_token();
        }

        res
    }
}

pub(crate) struct Ticket {
    pool: Option<Arc<InnerPool>>,
    pool_id: usize,
}

impl Ticket {
    pub(crate) fn new(pool: Arc<InnerPool>) -> Self {
        Ticket {
            pool: Some(pool),
            pool_id: 0,
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        // if we still own the reference to the poll, it means we need to return the token to the
        // pool. A Lannister never forgets his or her debts!
        if let Some(pool) = self.pool.take() {
            pool.return_token();

            TOKEN_BUCKET.with(|set| {
                set.borrow_mut().remove(&self.pool_id);
            });
        }
    }
}

impl Future for Ticket {
    type Output = ();

    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        assert!(
            self.pool.is_some(),
            "The pass has already been issued, yet the ticket is polled for access again ... "
        );

        let ref_this = self.get_mut();

        // retract the pool and current pool's id, to identify the gatekeeper
        let pool = ref_this.pool.take().unwrap();
        let pool_id = pool.get_id();

        // check if the parent future has already obtained the permit
        let need_token = TOKEN_BUCKET.with(|set| !set.borrow().contains(&pool_id));

        if !need_token || pool.wait_for_token(true) {
            // if the token is obtained by self, we still need the reference to the poll
            // when returning the token; also register that we have the token so the children
            // futures, if any, won't bother to get (i.e. waste) another token.
            if need_token {
                ref_this.pool.replace(pool);
                ref_this.pool_id = pool_id;

                TOKEN_BUCKET.with(|set| {
                    set.borrow_mut().insert(pool_id);
                });
            }

            // if we got the token, create the TokenPass with the reference to the pool
            return Poll::Ready(());
        }

        // we can't get a token yet, make sure correct context are set, then we will go to sleep.
        pool.enqueue(ctx.waker().clone());
        ref_this.pool.replace(pool);

        Poll::Pending
    }
}
