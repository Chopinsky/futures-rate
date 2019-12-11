use crate::semaphore::{InnerPool, TokenFetcher};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

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
    pub(crate) fn new(fut: F, pool: Arc<InnerPool>) -> Permit<R, F> {
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
        let (fut, pool) = unsafe {
            let ptr = Pin::get_unchecked_mut(self);

            (
                Pin::new_unchecked(&mut ptr.fut),
                Pin::new_unchecked(&ptr.pool),
            )
        };

        pool.wait_for_token();
        let res = fut.poll(ctx);
        pool.return_token();

        res
    }
}
