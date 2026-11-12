//! The one tokio runtime every Toasty database runs on.
use std::cell::Cell;
use std::future::Future;
use std::sync::OnceLock;

thread_local! {
    /// Set on the ORM runtime's own worker threads.
    static ORM_WORKER: Cell<bool> = const { Cell::new(false) };
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("gespan-orm")
            .enable_all()
            .on_thread_start(|| ORM_WORKER.with(|worker| worker.set(true)))
            .build()
            .expect("the ORM runtime starts")
    })
}

/// Runs a Toasty future to completion from synchronous code.
///
/// Safe to call from any thread, including one already inside another tokio
/// runtime (the mobile server's, say): there the future runs on a scoped
/// helper thread that waits on the ORM runtime, so `block_on` is never called
/// from a runtime thread. Calling `run` from inside a future that `run` is
/// already driving is a bug and panics, since it could starve the runtime.
pub fn run<F>(future: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    assert!(
        !ORM_WORKER.with(Cell::get),
        "orm::run called from inside the ORM runtime; await the future instead"
    );
    let runtime = runtime();
    if tokio::runtime::Handle::try_current().is_err() {
        return runtime.block_on(future);
    }
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("gespan-orm-wait".into())
            .spawn_scoped(scope, || runtime.block_on(future))
            .expect("the ORM wait thread starts")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}
