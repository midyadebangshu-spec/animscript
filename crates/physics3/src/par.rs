//! Deterministic data parallelism: the index range is cut into contiguous chunks, each chunk is
//! computed on its own thread, and the results are joined in index order. Every element is computed
//! by the same code from the same inputs whatever the thread count, so the output does not depend
//! on it.
use std::ops::Range;

/// The thread count to use: `threads` if given, else the machine's parallelism.
pub fn thread_count(threads: usize) -> usize {
    if threads > 0 {
        threads
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    }
}

/// Run `f` over contiguous chunks of `0..n`; the results come back in chunk order.
pub fn par_ranges<R, F>(n: usize, threads: usize, f: F) -> Vec<R>
where
    R: Send,
    F: Fn(Range<usize>) -> R + Sync,
{
    let t = thread_count(threads).min(n / 256 + 1).max(1);
    if t == 1 {
        return vec![f(0..n)];
    }
    let per = n.div_ceil(t);
    let ranges: Vec<Range<usize>> = (0..t)
        .map(|k| (k * per).min(n)..((k + 1) * per).min(n))
        .collect();
    std::thread::scope(|s| {
        let handles: Vec<_> = ranges
            .iter()
            .map(|r| {
                let r = r.clone();
                let f = &f;
                s.spawn(move || f(r))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a fluid worker panicked"))
            .collect()
    })
}

/// `f(i)` for every `i` in `0..n`, in order.
pub fn par_map<T, F>(n: usize, threads: usize, f: F) -> Vec<T>
where
    T: Send,
    F: Fn(usize) -> T + Sync,
{
    par_ranges(n, threads, |r| r.map(&f).collect::<Vec<T>>())
        .into_iter()
        .flatten()
        .collect()
}
