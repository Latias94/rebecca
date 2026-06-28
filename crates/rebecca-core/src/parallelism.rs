use std::sync::OnceLock;

use rayon::{ThreadPool, ThreadPoolBuilder};

pub(crate) fn bounded_parallelism_budget() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().clamp(2, 8))
        .unwrap_or(2)
}

pub(crate) fn run_scoped_parallel_work<R, F>(
    pool: &'static OnceLock<ThreadPool>,
    pool_name: &'static str,
    work: F,
) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    bounded_thread_pool(pool, pool_name).install(work)
}

fn bounded_thread_pool(
    pool: &'static OnceLock<ThreadPool>,
    pool_name: &'static str,
) -> &'static ThreadPool {
    pool.get_or_init(|| {
        ThreadPoolBuilder::new()
            .num_threads(bounded_parallelism_budget())
            .build()
            .unwrap_or_else(|_| panic!("failed to build Rebecca {pool_name} thread pool"))
    })
}

#[cfg(test)]
mod tests {
    use super::{bounded_parallelism_budget, run_scoped_parallel_work};
    use std::sync::Arc;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rayon::ThreadPool;

    #[test]
    fn bounded_parallelism_budget_stays_bounded() {
        let budget = bounded_parallelism_budget();

        assert!((2..=8).contains(&budget));
    }

    #[test]
    fn run_scoped_parallel_work_executes_work() {
        static POOL: OnceLock<ThreadPool> = OnceLock::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_ref = Arc::clone(&counter);

        run_scoped_parallel_work(&POOL, "test", move || {
            counter_ref.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
