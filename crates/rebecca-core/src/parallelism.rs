pub(crate) fn bounded_parallelism_budget() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().clamp(2, 8))
        .unwrap_or(2)
}
