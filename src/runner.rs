use std::sync::OnceLock;

use jxl::api::{JxlParallelRunner, JxlParallelRunnerFun};
use rayon::iter::{IntoParallelIterator, ParallelIterator};

static THREAD_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

fn get_thread_pool() -> &'static rayon::ThreadPool {
    THREAD_POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(8);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
    })
}

pub struct RayonParallelRunner;

impl JxlParallelRunner for RayonParallelRunner {
    fn run(&mut self, num: usize, fun: &JxlParallelRunnerFun) -> jxl::error::Result<()> {
        if num == 1 {
            for i in 0..num {
                fun(i)?;
            }
            return Ok(());
        }

        let pool = get_thread_pool();

        if pool.current_num_threads() == 1 {
            for i in 0..num {
                fun(i)?;
            }
            return Ok(());
        }

        pool.install(|| (0..num).into_par_iter().try_for_each(fun))
    }

    fn num_threads(&self) -> usize {
        get_thread_pool().current_num_threads()
    }
}
