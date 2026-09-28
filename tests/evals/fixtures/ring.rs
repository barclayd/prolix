use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fixed-capacity single-producer, single-consumer queue.
///
/// `push` and `pop` never allocate or block, so both are safe to call from an
/// audio callback. `capacity` must be a power of two.
pub struct Ring<T> {
    buf: Box<[UnsafeCell<MaybeUninit<T>>]>,
    // Monotonic counters, masked on access. Wrapping is harmless because the
    // capacity divides 2^usize::BITS.
    head: AtomicUsize,
    tail: AtomicUsize,
}

// Layout follows Dmitry Vyukov's bounded queue:
// https://www.1024cores.net/home/lock-free-algorithms/queues/bounded-mpmc-queue
impl<T> Ring<T> {
    fn slot(&self, i: usize) -> *mut MaybeUninit<T> {
        // Same as i % len, since len is a power of two
        self.buf[i & (self.buf.len() - 1)].get()
    }

    pub fn push(&self, value: T) -> Result<(), T> {
        let tail = self.tail.load(Ordering::Relaxed);
        // Acquire pairs with the Release store in pop(), so the slot we are about
        // to overwrite has been fully read by the consumer.
        let head = self.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) == self.buf.len() {
            return Err(value);
        }
        // SAFETY: only the producer writes this slot, and the consumer can't see it
        // until the Release store below publishes the new tail.
        unsafe { (*self.slot(tail)).write(value) };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }
}
