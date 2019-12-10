# futures-rate

This library provides easy tools to help Rust applications guide
critical resources or code paths from being overwhelmed. 

Depending on the configuration, the library will limit the amount
of guarded futures being polled concurrently.   