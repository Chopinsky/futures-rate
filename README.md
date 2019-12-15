[futures-rate][docsrs]
======================

[![futures-rate on crates.io][cratesio-image]][cratesio]
[![futures-rate on docs.rs][docsrs-image]][docsrs]

[cratesio]: https://crates.io/crates/futures-rate
[cratesio-image]: https://img.shields.io/crates/v/futures-rate.svg
[docsrs-image]: https://docs.rs/futures-rate/badge.svg
[docsrs]: https://docs.rs/futures-rate

## What is this library

This library provides easy tools to help Rust applications guide
critical resources or code paths from being overwhelmed. 

Depending on the configuration, the library will limit the amount
of guarded futures being polled concurrently.

## How to use

#### Installation

First of all, add the dependency to your Rust project:

```bash
$ cargo install futures-rate
``` 

Or in your project's `Cargo.toml`, add the following dependency:

```toml
[dependencies]
futures-rate = "^0.1.0"
```

then run `$ cargo install` in your terminal from project's root directory.

#### Limit access rate

* Create and manage a [`GateKeeper`] object in your main thread, which will
set the access limit to a certain resource:

```rust
use futures_rate::GateKeeper;

/// then in main thread
fn main() {
    // ... other code

    // At most 10 futures can pass the gate at any given time  
    let gatekeeper = GateKeeper::new(10);

    // ... more code
}
```

* Then register your future to the [`GateKeeper`] such that the resourceful
future can be protected:

```rust
use futures_rate::{GateKeeper, Permit};

/// in the business logic which has access to the `gatekeeper` object
async fn work(gatekeeper: &GateKeeper) -> usize {
    // create the IO-heavy future
    let ioFut = async { 
        // do async work here 
    };

    let permit = gatekeeper.register(async {
        // At most 10 IO work can be on-the-fly at any given time
        ioFut.await;        
        
        // the result of all questions is always 42
        42
    });
    
    permit.await
}
``` 

#### Naive Future Lock

If setting the 


## Examples

A classic use case is to place a [`GateKeeper`] over an a client socket pool, such that only
a limited number of future visitors can be allowed to poll the resources and hence limit the
amount of open connections.

 ```rust
 use futures::{executor, future};
 use futures_rate::{GateKeeper, Permit};
 use std::future::Future;
 use std::thread;
 use std::time::Duration;

 fn main() {
     let gatekeeper = GateKeeper::new(1);
     let fut_values = async {
         let fut_1 = build_fut(0, &gatekeeper);
         let fut_2 = build_fut(1, &gatekeeper);
         let fin = future::join(fut_1, fut_2);
         fin.await
     };

     let values = executor::block_on(fut_values);

     println!("Values from fut_1={:?}", values.0);
     println!("Values from fut_2={:?}", values.1);
 }

 fn build_fut(
     offset: i32,
     gatekeeper: &GateKeeper,
 ) -> Permit<Vec<i32>, impl Future<Output = Vec<i32>>> {
     gatekeeper.register(async move {
         let mut values = Vec::with_capacity(100);
         (0..100).for_each(|v| {
             thread::sleep(Duration::from_millis(1));
             values.push(2 * v + offset);
         });

         values
     }).unwrap()
 }
 ```

 [`GateKeeper`]: struct.GateKeeper.html
