use crate::inner::{InnerPool, TokenFetcher};
use crate::{InterruptedReason, SpinPolicy};
use std::cell::RefCell;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;

thread_local!(
    static PERMIT_SET: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
);

pub(crate) enum Envelope<R>
where
    R: Send + 'static,
{
    Stub(TicketStub),
    Output(R),
}

pub(crate) trait TokenHolder {
    fn render_token(&mut self);
}

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

        // now we're ready to return the TicketStub, if we're the one requested it
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
    pending_count: usize,
    token_obtained: bool,
    pool: Option<Arc<InnerPool>>,
    fut: Option<Pin<Box<F>>>,
    spin_policy: SpinPolicy,
}

impl<R, F> Ticket<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    pub(crate) fn new(pool: Arc<InnerPool>, fut: Option<F>) -> Self {
        Ticket {
            pool_id: 0,
            pending_count: 0,
            token_obtained: false,
            pool: Some(pool),
            fut: fut.map(Box::pin),
            spin_policy: SpinPolicy::InplaceWait,
        }
    }

    pub(crate) fn set_spin_policy(&mut self, spin: SpinPolicy) {
        self.spin_policy = spin;
    }

    fn request_token(&mut self) -> bool {
        assert!(
            !self.token_obtained,
            "failed to return previously obtained token ... "
        );

        if let Some(pool) = self.pool.as_mut() {
            if pool.request_token(true) {
                let pool_id = pool.get_id();

                PERMIT_SET.with(|set| {
                    set.borrow_mut().insert(pool_id);
                });

                self.pool_id = pool_id;
                self.token_obtained = true;

                return true;
            }
        }

        false
    }

    fn make_stub(&mut self) -> TicketStub {
        // now the token has been transferred to the `stub`, we no longer own the token, and we
        // won't need to render the token from the drop function.
        self.token_obtained = false;

        // generate the stub from the ticket
        TicketStub {
            pool: self.pool.take().unwrap(),
        }
    }
}

impl<R, F> TokenHolder for Ticket<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    fn render_token(&mut self) {
        if let Some(pool) = self.pool.as_ref() {
            // if we own the reference to the pool, meaning we also own the token that we took, so
            // we can return it now.
            assert!(
                self.token_obtained,
                "failed at double-returning a previously obtained token ... "
            );

            pool.return_token();

            PERMIT_SET.with(|set| {
                set.borrow_mut().remove(&self.pool_id);
            });

            self.token_obtained = false;
        }
    }
}

impl<R, F> Drop for Ticket<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    fn drop(&mut self) {
        self.render_token();
    }
}

impl<R, F> Future for Ticket<R, F>
where
    R: Send + 'static,
    F: Future<Output = R> + 'static,
{
    type Output = Result<Envelope<R>, InterruptedReason>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        assert!(
            self.pool.is_some(),
            "The pass has already been issued, yet the ticket is polled for access again ... "
        );

        //TODO: check remote_control, if interrupted, return
        //       Poll::Ready(Err(InterruptedReason::Cancelled));

        let ref_this = self.get_mut();

        // check if the parent future has already obtained the permit
        let need_token = if ref_this.pool_id == 0 {
            true
        } else {
            PERMIT_SET.with(|set| !set.borrow().contains(&ref_this.pool_id))
        };

        if !need_token || ref_this.request_token() {
            // if we own the future (i.e. we're in the `TicketStubPolicy::Cooperative` mode), poll
            // the future and return from what we got. Either way, the TicketStub will be returned
            // afterwards -- either explicitly when results are pending, or implicitly when the
            // `Ticket` goes out of the scope and return the TicketStub while being dropped.
            if let Some(fut_info) = ref_this.fut.as_mut() {
                let result = abort_on_panic(|| fut_info.as_mut().poll(ctx));

                return match result {
                    Poll::Pending => {
                        let spin = ref_this.spin_policy != SpinPolicy::None;

                        // this is the key move for the cooperative mode: we return the token
                        // immediately after a pending result, meaning we won't wait for the future
                        // to complete before returning the token.
                        if need_token {
                            ref_this.render_token();

                            if spin {
                                ref_this.pending_count += 1;
                            }
                        }

                        if ref_this.pending_count >= 4 && spin {
                            if ref_this.spin_policy == SpinPolicy::Yield {
                                thread::yield_now();
                            } else {
                                thread::sleep(Duration::from_micros((1 << ref_this.pending_count) as u64));
                            }

                            if ref_this.pending_count >= 10 {
                                ref_this.pending_count = 4;
                            }
                        }

                        Poll::Pending
                    }
                    Poll::Ready(val) => {
                        // the future is done, we will return the result. the token will be returned
                        Poll::Ready(Ok(Envelope::Output(val)))
                    }
                };
            }

            // if running in `TicketStubPolicy::Preemptive` mode, we get the TicketStub, and we
            // create the TicketStubPass with the reference to the pool, and we will run the future
            // to the completion. The TicketStub will be returned when the `Ticket` goes out of the
            // scope after the future is completed. This is the key move, since we generated a stub
            // from the token, which will live until the future (owned by the intermediate future
            // generator between gatekeeper and the ticket) is moved towards completion.
            return Poll::Ready(Ok(Envelope::Stub(ref_this.make_stub())));
        }

        // we can't get a token yet, make sure correct context are set, then we will go back to wait.
        if let Some(pool) = ref_this.pool.as_ref() {
            // only enqueue to wake up if we're in the preemptive mode; otherwise the owning future
            // will wake us up
            pool.enqueue(ctx.waker().clone());
        }

        Poll::Pending
    }
}

pub(crate) struct TicketStub {
    pool: Arc<InnerPool>,
}

impl TokenHolder for TicketStub {
    fn render_token(&mut self) {
        self.pool.return_token();
    }
}

impl Drop for TicketStub {
    fn drop(&mut self) {
        // if we still own the reference to the poll, it means we need to return the TicketStub to the
        // pool. A Lannister never forgets his or her debts!
        self.render_token();
    }
}

#[inline]
fn abort_on_panic<R>(f: impl FnOnce() -> R) -> R {
    struct Bomb;

    impl Drop for Bomb {
        fn drop(&mut self) {
            //TODO: less aggressive here
            std::process::abort();
        }
    }

    let bomb = Bomb;
    let t = f();
    std::mem::forget(bomb);

    t
}
