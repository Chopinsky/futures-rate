mod enter;
mod permit;
mod semaphore;
mod threads_queue;

pub use semaphore::Semaphore;
pub use permit::Permit;

pub mod prelude {
    pub use crate::semaphore::Semaphore;
    pub use crate::permit::Permit;
}

#[derive(Debug, Copy, Clone, PartialOrd, PartialEq)]
pub enum RatioType {
    /// A cap of the number of the maximum number of tokens available at any given time.
    Static(usize),

    /// A fixed number of tokens that will become available at the beginning of every millisecond.
    /// Note that this number will *not* count or include the ones not yet returned by the holders.
    Fixed(usize),
}
