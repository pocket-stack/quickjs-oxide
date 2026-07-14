use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, PoisonError};
use std::thread;

/// Runs indexed jobs with at most `workers` active coordinator threads.
///
/// Successful values are returned in job-index order, independently of their
/// completion order. An infrastructure error stops workers from starting more
/// jobs, but every job that already started is joined before the lowest-indexed
/// error is returned.
pub(super) fn run_bounded<T, F>(job_count: usize, workers: usize, run: F) -> Result<Vec<T>, String>
where
    T: Send,
    F: Fn(usize) -> Result<T, String> + Sync,
{
    if workers == 0 {
        return Err("worker count must be greater than zero".to_owned());
    }
    if job_count == 0 {
        return Ok(Vec::new());
    }

    struct Dispatch {
        next_job: usize,
        stopped: bool,
    }

    let worker_count = workers.min(job_count);
    let dispatch = Mutex::new(Dispatch {
        next_job: 0,
        stopped: false,
    });
    let batches = thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let dispatch = &dispatch;
            let run = &run;
            handles.push(scope.spawn(move || {
                let mut completed = Vec::new();
                loop {
                    let index = {
                        let mut state = dispatch.lock().unwrap_or_else(PoisonError::into_inner);
                        if state.stopped || state.next_job >= job_count {
                            None
                        } else {
                            let index = state.next_job;
                            state.next_job += 1;
                            Some(index)
                        }
                    };
                    let Some(index) = index else {
                        break;
                    };

                    let result = panic::catch_unwind(AssertUnwindSafe(|| run(index)))
                        .unwrap_or_else(|_| {
                            Err(format!(
                                "scheduler worker panicked while running job {index}"
                            ))
                        });
                    let failed = result.is_err();
                    completed.push((index, result));
                    if failed {
                        dispatch
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .stopped = true;
                        break;
                    }
                }
                completed
            }));
        }

        let mut batches = Vec::with_capacity(worker_count);
        let mut thread_panicked = false;
        for handle in handles {
            match handle.join() {
                Ok(completed) => batches.push(completed),
                Err(_) => thread_panicked = true,
            }
        }
        if thread_panicked {
            Err("scheduler coordinator thread panicked".to_owned())
        } else {
            Ok(batches)
        }
    })?;

    let mut slots = (0..job_count)
        .map(|_| None)
        .collect::<Vec<Option<Result<T, String>>>>();
    for (index, result) in batches.into_iter().flatten() {
        let Some(slot) = slots.get_mut(index) else {
            return Err(format!("scheduler returned invalid job index {index}"));
        };
        if slot.replace(result).is_some() {
            return Err(format!("scheduler returned duplicate job index {index}"));
        }
    }

    let mut output = Vec::with_capacity(job_count);
    for (index, slot) in slots.into_iter().enumerate() {
        match slot {
            Some(Ok(value)) => output.push(value),
            Some(Err(error)) => return Err(error),
            None => {
                return Err(format!(
                    "scheduler stopped without a result for job {index}"
                ));
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use super::run_bounded;

    #[test]
    fn results_follow_job_order_and_parallelism_is_bounded() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let first_wave = Barrier::new(3);
        let results = run_bounded(12, 3, |index| {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            if index < 3 {
                first_wave.wait();
                thread::sleep(Duration::from_millis((2 - index) as u64 * 10));
            }
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(index)
        })
        .unwrap();

        assert_eq!(results, (0..12).collect::<Vec<_>>());
        assert_eq!(peak.load(Ordering::SeqCst), 3);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn zero_jobs_do_not_invoke_the_runner() {
        let called = AtomicBool::new(false);
        let results = run_bounded(0, 4, |_| {
            called.store(true, Ordering::SeqCst);
            Ok(0usize)
        })
        .unwrap();

        assert!(results.is_empty());
        assert!(!called.load(Ordering::SeqCst));
    }

    #[test]
    fn one_worker_runs_jobs_in_index_order() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let results = run_bounded(6, 1, |index| {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(index * 2)
        })
        .unwrap();

        assert_eq!(results, vec![0, 2, 4, 6, 8, 10]);
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn errors_stop_new_jobs_and_use_the_lowest_started_index() {
        let started = AtomicUsize::new(0);
        let first_wave = Barrier::new(4);
        let error = run_bounded(64, 4, |index| -> Result<usize, String> {
            started.fetch_add(1, Ordering::SeqCst);
            if index < 4 {
                first_wave.wait();
            }
            match index {
                1 => {
                    thread::sleep(Duration::from_millis(10));
                    Err("error from job 1".to_owned())
                }
                3 => Err("error from job 3".to_owned()),
                _ => {
                    thread::sleep(Duration::from_millis(30));
                    Ok(index)
                }
            }
        })
        .unwrap_err();

        assert_eq!(error, "error from job 1");
        assert_eq!(started.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn zero_workers_are_rejected() {
        let error = run_bounded(1, 0, |_| Ok(0usize)).unwrap_err();
        assert_eq!(error, "worker count must be greater than zero");
    }
}
