use crate::token_pool::{InnerPool, TokenFetcher};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

pub struct Semaphore<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    fut: F,
    pool: Arc<InnerPool>,
}

impl<R, F> Semaphore<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    pub(crate) fn new(fut: F, pool: Arc<InnerPool>) -> Semaphore<R, F> {
        Semaphore { fut, pool }
    }
}

impl<R, F> Future for Semaphore<R, F>
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

        let res = match fut.poll(ctx) {
            Poll::Ready(val) => Poll::Ready(val),
            Poll::Pending => Poll::Pending,
        };

        pool.return_token();
        res
    }
}
