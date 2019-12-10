use crossbeam_queue::SegQueue;
use std::thread::Thread;

pub(crate) struct ThreadsQueue(SegQueue<Thread>);

impl ThreadsQueue {
    pub(crate) fn new() -> Self {
        ThreadsQueue(SegQueue::new())
    }

    pub(crate) fn enqueue(&self, t: Thread) {
        self.0.push(t);
    }

    pub(crate) fn dequeue(&self) -> Option<Thread> {
        self.0.pop().ok()
    }
}