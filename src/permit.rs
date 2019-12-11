use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::cell::RefCell;
use std::collections::HashSet;
use crate::gatekeeper::{InnerPool, TokenFetcher};

thread_local!(
    static PERMIT_SET: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
);

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

            (
                Pin::new_unchecked(&mut ptr.fut),
                &ptr.pool,
            )
        };

        // the current pool's id, to identify the gatekeeper
        let pool_id = pool.get_id();

        // check if the parent future has already obtained the permit
        let need_token = PERMIT_SET.with(|set| {
            !set.borrow().contains(&pool_id)
        });

        // if we're the first future to try the gatekeeper, wait for a permit to be available
        if need_token {
            pool.wait_for_token(false);

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