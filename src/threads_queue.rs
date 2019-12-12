use crossbeam_queue::SegQueue;
use std::task::Waker;

/*
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
*/

pub(crate) struct WaitingList(SegQueue<Waker>);

impl WaitingList {
    pub(crate) fn new() -> Self {
        WaitingList(SegQueue::new())
    }

    pub(crate) fn enqueue(&self, w: Waker) {
        self.0.push(w);
    }

    pub(crate) fn dequeue(&self) -> Option<Waker> {
        self.0.pop().ok()
    }
}
