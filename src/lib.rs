mod enter;
mod threads_queue;

pub mod semaphore;
pub mod token_pool;

enum RatioType {
    Static(usize),
    Rate(usize),
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
