use std::cell::RefCell;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use crate::inner::{InnerPool, TokenFetcher};
use crate::InterruptedReason;

thread_local!(
    static PERMIT_SET: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
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
        let need_token = PERMIT_SET.with(|set| !set.borrow().contains(&pool_id));

        // if we're the first future to try the gatekeeper, wait for a permit to be available
        if need_token {
            pool.request_token(false);

            PERMIT_SET.with(|set| {
                (*set.borrow_mut()).insert(pool_id);
            });
        }

        // poll the future, do the actual work
        let res = fut.poll(ctx);

        // now we're ready to return the token, if we're the one requested it
        if need_token {
            PERMIT_SET.with(|set| {
                (*set.borrow_mut()).remove(&pool_id);
            });

            pool.return_token();
        }

        res
    }
}

pub(crate) struct Ticket<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    pool_id: usize,
    pool: Option<Arc<InnerPool>>,
    fut: Option<Pin<Box<F>>>,
}

impl<R, F> Ticket<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    pub(crate) fn new(pool: Arc<InnerPool>, fut: Option<F>) -> Self {
        Ticket {
            pool_id: 0,
            pool: Some(pool),
            fut: fut.map(Box::pin),
        }
    }

    fn return_token(&mut self) {
        if let Some(pool) = self.pool.take() {
            pool.return_token();

            PERMIT_SET.with(|set| {
                set.borrow_mut().remove(&self.pool_id);
            });
        }
    }
}

impl<R, F> Drop for Ticket<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    fn drop(&mut self) {
        // if we still own the reference to the poll, it means we need to return the token to the
        // pool. A Lannister never forgets his or her debts!
        self.return_token();
    }
}

impl<R, F> Future for Ticket<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    type Output = Result<Option<R>, InterruptedReason>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        assert!(
            self.pool.is_some(),
            "The pass has already been issued, yet the ticket is polled for access again ... "
        );

        //TODO: check token, if interrupted, return Poll::Ready(Err(InterruptedReason::Cancelled));

        let ref_this = self.get_mut();

        // retract the pool and current pool's id, to identify the gatekeeper
        let pool = ref_this.pool.take().unwrap();
        let pool_id = pool.get_id();

        // check if the parent future has already obtained the permit
        let need_token = PERMIT_SET.with(|set| !set.borrow().contains(&pool_id));

        if !need_token || pool.request_token(true) {
            // if the token is obtained by self, we still need the reference to the poll
            // when returning the token; also register that we have the token so the children
            // futures, if any, won't bother to get (i.e. waste) another token.
            if need_token {
                PERMIT_SET.with(|set| {
                    set.borrow_mut().insert(pool_id);
                });

                ref_this.pool.replace(pool);
                ref_this.pool_id = pool_id;
            }

            // if we own the future (i.e. we're in the `TokenPolicy::Cooperative` mode), poll the future
            // and return from what we got. Either way, the token will be returned afterwards -- either
            // explicitly when results are pending, or implicitly when the `Ticket` goes out of the
            // scope and return the token while being dropped.
            if let Some(fut) = ref_this.fut.as_mut() {
                return match fut.as_mut().poll(ctx) {
                    Poll::Pending => {
                        // if we've requested a token, return it now
                        if need_token {
                            ref_this.return_token();
                        }

                        Poll::Pending
                    }
                    Poll::Ready(val) => Poll::Ready(Ok(Some(val))),
                };
            }

            // if running in `TokenPolicy::Preemptive` mode, we get the token, and we create the
            // TokenPass with the reference to the pool, and we will run the future to the completion.
            // The token will be returned when the `Ticket` goes out of the scope after the future
            // is completed.
            return Poll::Ready(Ok(None));
        }

        // we can't get a token yet, make sure correct context are set, then we will go to sleep.
        pool.enqueue(ctx.waker().clone());
        ref_this.pool.replace(pool);

        Poll::Pending
    }
}

pub struct Token {}
