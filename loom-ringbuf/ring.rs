use super::*;

use std::mem::MaybeUninit;
use loom::cell::UnsafeCell;
use loom::sync::{atomic, Arc};


struct Ring<T> {
    buf: Box<[UnsafeCell<MaybeUninit<T>>]>,
    flags: Box<[atomic::AtomicU64]>,
    head: atomic::AtomicU32,
    tail: atomic::AtomicU32,
    epoch: atomic::AtomicU64
}

unsafe impl<T: Send> Send for Ring<T> {}
unsafe impl<T: Sync> Sync for Ring<T> {}

#[derive(Clone)]
pub struct Producer<T>(Arc<Ring<T>>);
pub struct Consumer<T>(Arc<Ring<T>>);

pub fn new<T: Copy>(size: usize) -> (Producer<T>, Consumer<T>) {
    assert!(size.is_power_of_two());

    let mut buf = Vec::with_capacity(size);
    let mut buf2 = Vec::with_capacity(size);

    for _ in 0..size {
        buf.push(UnsafeCell::new(MaybeUninit::uninit()));
        buf2.push(atomic::AtomicU64::new(0));
    }

    let ring = Arc::new(Ring {
        buf: buf.into_boxed_slice(),
        flags: buf2.into_boxed_slice(),
        head: atomic::AtomicU32::new(0),
        tail: atomic::AtomicU32::new(0),
        epoch: atomic::AtomicU64::new(0)
    });

    let producer = Producer(ring.clone());
    let consumer = Consumer(ring);

    (producer, consumer)
}

impl<T: Copy> Producer<T> {
    pub fn push(&self, t: T) -> Result<(), T> {
        let mask = (self.0.buf.len() - 1) as u32;
        let mut head = self.0.head.load(atomic::Ordering::Acquire);
        let mut tail = self.0.tail.load(atomic::Ordering::Acquire);
        let mut epoch = self.0.epoch.load(atomic::Ordering::Acquire);
        let mut tail2 = tail;
        let mut is_flags_failed = false;
        let mut next_epoch_ready = true;

        loop {
            assert!(tail.wrapping_sub(head) <= self.0.buf.len() as u32,
                "{} {} {} {}",
                head,
                tail,
                tail2,
                epoch
            );

            if tail.wrapping_sub(head) == self.0.buf.len() as u32 {
                if head == tail2 {
                    head = self.0.head.load(atomic::Ordering::Acquire);
                    tail = self.0.tail.load(atomic::Ordering::Acquire);

                    if is_flags_failed && next_epoch_ready {
                        epoch = self.0.epoch.compare_exchange(
                            epoch,
                            epoch.wrapping_add(2),
                            atomic::Ordering::AcqRel,
                            atomic::Ordering::Acquire
                        )
                            .map(|epoch| epoch.wrapping_add(2))
                            .unwrap_or_else(|epoch| epoch);
                    } else {
                        epoch = self.0.epoch.load(atomic::Ordering::Acquire);
                    }

                    tail2 = tail;
                    is_flags_failed = false;
                    next_epoch_ready = true;

                    thread::yield_now();
                    continue
                }

                return Err(t);
            }

            match self.0.flags[(tail & mask) as usize]
                .compare_exchange(
                    epoch,
                    epoch.wrapping_add(1),
                    atomic::Ordering::AcqRel,
                    atomic::Ordering::Acquire
                )
            {
                Ok(_) => break,
                Err(epoch) => {
                    is_flags_failed = true;
                    next_epoch_ready &= epoch & 1 == 0;
                }
            }

            tail = tail.wrapping_add(1);

            thread::yield_now();
        }

        let index = (tail & mask) as usize;

        unsafe {
            self.0.buf[index]
                .with_mut(|p| (*p).as_mut_ptr().write(t));
        }

        while self.0.tail.compare_exchange_weak(
            tail,
            tail.wrapping_add(1),
            atomic::Ordering::Release,
            atomic::Ordering::Relaxed
        ).is_err() {
            thread::yield_now();
        }

        self.0.flags[index]
            .fetch_add(1, atomic::Ordering::Release);

        Ok(())
    }
}

impl<T: Copy> Consumer<T> {
    pub fn pop(&mut self) -> Option<T> {
        let mask = (self.0.buf.len() - 1) as u32;
        let head = unsafe { load_u32(&self.0.head) };
        let tail = self.0.tail.load(atomic::Ordering::Acquire);

        // dbg!(head, tail);

        if head == tail {
            return None;
        }

        let t = self.0.buf[(head & mask) as usize].with(|p| unsafe { p.read().assume_init() });

        self.0.head.store(head.wrapping_add(1), atomic::Ordering::Release);

        Some(t)
    }
}

unsafe fn load_u32(t: &atomic::AtomicU32) -> u32 {
    #[cfg(feature = "loom")] {
        t.unsync_load()
    }

    #[cfg(not(feature = "loom"))] {
        (t as *const atomic::AtomicU32).read().into_inner()
    }
}
