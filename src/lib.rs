mod enter;
mod semaphore;
mod threads_queue;
mod token_pool;

pub use semaphore::Semaphore;
pub use token_pool::TokenPool;

pub mod prelude {
    pub use crate::semaphore::Semaphore;
    pub use crate::TokenPool;
}

#[derive(PartialOrd, PartialEq)]
pub enum RatioType {
    /// A cap of the number of the maximum number of tokens available at any given time.
    Static(usize),

    /// A fixed number of tokens that will become available at the beginning of every millisecond.
    /// Note that this number will *not* count or include the ones not yet returned by the holders.
    Fixed(usize),
}
