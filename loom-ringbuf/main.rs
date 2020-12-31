#[allow(dead_code)]
#[cfg(not(feature = "loom"))]
mod loom {
    pub use std::sync;
    pub use std::thread;

    pub mod cell {
        pub struct UnsafeCell<T>(std::cell::UnsafeCell<T>);

        impl<T> UnsafeCell<T> {
            #[inline(always)]
            pub fn new(t: T) -> UnsafeCell<T> {
                UnsafeCell(std::cell::UnsafeCell::new(t))
            }

            #[inline(always)]
            pub fn with<F, R>(&self, f: F) -> R
            where F: FnOnce(*const T) -> R
            {
                f(self.0.get())
            }

            #[inline(always)]
            pub fn with_mut<F, R>(&self, f: F) -> R
            where F: FnOnce(*mut T) -> R
            {
                f(self.0.get())
            }
        }
    }

    pub fn model<F>(f: F)
    where
        F: Fn() + Sync + Send + 'static
    {
        f()
    }
}

mod ring;

use loom::thread;

fn mpsc(size: usize, thread: usize, count: usize) {
    loom::model(move || {
        let (producer, mut consumer) = ring::new::<u32>(size);

        let mut joins = Vec::new();
        for x in 0..thread {
            let producer = producer.clone();
            let j = thread::spawn(move || {
                let mut n = 0;
                while n < count {
                    if producer.push((x + n) as u32).is_ok() {
                        n += 1;
                    }

                    thread::yield_now();
                }

                #[cfg(not(feature = "loom"))]
                println!("{} end", x);
            });
            joins.push(j);
        }

        let mut n = 0;
        while n < (thread * count) {
            if consumer.pop().is_some() {
                n += 1;
            }

            thread::yield_now();
        }

        for j in joins {
            j.join().unwrap();
        }

        // println!("test end");
    });
}

fn main() {
    #[cfg(feature = "loom")]
    mpsc(4, 3, 2);

    #[cfg(not(feature = "loom"))]
    mpsc(8, 4, 1024);
}
