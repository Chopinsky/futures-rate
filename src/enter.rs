use std::cell::Cell;

thread_local!(
    static ENTERED: Cell<usize> = Cell::new(0)
);

pub(crate) fn arrive() {
    ENTERED.with(|c| {
        c.set(c.get() + 1);
    })
}

pub(crate) fn depart() {
    ENTERED.with(|c| {
        c.set(c.get() - 1);
    })
}

pub(crate) fn test() -> bool {
    ENTERED.with(|c| c.get() > 0)
}
