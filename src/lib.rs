//! This library provides easy to use abstractions to guard critical code path or resources in the
//! futures and only allows a desired number of futures to be on-the-fly at any given time.
//!
//! A classic use case is to place a [`GateKeeper`] over an a client socket pool, such that only
//! a limited number of future visitors can be allowed to poll the resources and hence limit the
//! amount of open connections.
//!
//! # Example
//! ```rust
//! use futures::{executor, future};
//! use futures_rate::{GateKeeper, Permit};
//! use std::future::Future;
//! use std::thread;
//! use std::time::Duration;
//!
//! fn main() {
//!     let gatekeeper = GateKeeper::new(1);
//!     let fut_values = async {
//!         let fut_1 = build_fut(0, &gatekeeper);
//!         let fut_2 = build_fut(1, &gatekeeper);
//!         let fin = future::join(fut_1, fut_2);
//!         fin.await
//!     };
//!
//!     let values = executor::block_on(fut_values);
//!
//!     println!("Values from fut_1={:?}", values.0);
//!     println!("Values from fut_2={:?}", values.1);
//! }
//!
//! fn build_fut(
//!     offset: i32,
//!     gatekeeper: &GateKeeper,
//! ) -> Permit<Vec<i32>, impl Future<Output = Vec<i32>>> {
//!     gatekeeper.register(async move {
//!         let mut values = Vec::with_capacity(100);
//!         (0..100).for_each(|v| {
//!             thread::sleep(Duration::from_millis(1));
//!             values.push(2 * v + offset);
//!         });
//!
//!         values
//!     }).unwrap()
//! }
//! ```
//!
//! [`GateKeeper`]: struct.GateKeeper.html
//!

mod enter;
mod gatekeeper;
mod permit;
mod threads_queue;

pub use gatekeeper::GateKeeper;
pub use permit::Permit;

pub mod prelude {
    pub use crate::gatekeeper::GateKeeper;
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
