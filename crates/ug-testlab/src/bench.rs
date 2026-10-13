//! A short CPU benchmark.
//!
//! Not a score to compare against other machines' marketing numbers: it gives
//! the report a load the sensors must react to (load, clock, temperature,
//! power), and one-core and all-core figures that show whether the CPU is held
//! back (a low all-core/one-core ratio on a laptop usually means a power or
//! temperature limit). The work is integer mixing that the optimiser cannot
//! remove, and every thread checks its own result.

use std::time::{Duration, Instant};

/// Million rounds per second.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuScore {
    pub one_core: f64,
    pub all_cores: f64,
    pub threads: usize,
}

/// One round of xorshift and a multiply; returns the new state.
#[inline]
fn round(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

/// Rounds per second on this thread for `for_`. Returns (rate, final state).
fn spin(for_: Duration, seed: u64) -> (f64, u64) {
    let start = Instant::now();
    let mut x = seed | 1;
    let mut rounds: u64 = 0;
    loop {
        for _ in 0..200_000 {
            x = round(x);
        }
        rounds += 200_000;
        let t = start.elapsed();
        if t >= for_ {
            return (rounds as f64 / t.as_secs_f64(), x);
        }
    }
}

/// Checks the arithmetic on this core: a fixed number of rounds must end
/// on the same value every time (a CPU that is unstable under load, e.g.
/// undervolted too far, gets this wrong).
pub fn self_check() -> bool {
    let mut a = 1u64;
    let mut b = 1u64;
    for _ in 0..1_000_000 {
        a = round(a);
    }
    for _ in 0..1_000_000 {
        b = round(b);
    }
    a == b && a != 0
}

/// Runs the one-core part, then the all-core part. Blocking.
pub fn cpu(one: Duration, all: Duration) -> CpuScore {
    let (one_core, x) = spin(one, 0x9e37_79b9_7f4a_7c15);
    std::hint::black_box(x);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let rates: Vec<f64> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|i| s.spawn(move || spin(all, i as u64 + 7)))
            .collect();
        hs.into_iter()
            .filter_map(|h| h.join().ok())
            .map(|(r, x)| {
                std::hint::black_box(x);
                r
            })
            .collect()
    });
    CpuScore {
        one_core: one_core / 1e6,
        all_cores: rates.iter().sum::<f64>() / 1e6,
        threads,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_benchmark_counts_work_on_every_thread() {
        let s = cpu(Duration::from_millis(50), Duration::from_millis(50));
        assert!(s.one_core > 0.0 && s.all_cores > 0.0 && s.threads >= 1);
        assert!(self_check());
    }
}
