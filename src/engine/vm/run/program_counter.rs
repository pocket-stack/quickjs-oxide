//! Keep dispatch PCs local while the exclusive frame borrow is resident.
//! Drop materializes both PCs on normal, Result-error and Rust-unwind exits.
//! Observable release boundaries explicitly materialize fault before publishing
//! the runtime active PC. No callback receives the borrowed execution frame.

pub(super) struct ProgramCounter<'a> {
    pub fault: usize,
    pub resume: usize,
    published_fault: &'a mut usize,
    published_resume: &'a mut usize,
}
impl<'a> ProgramCounter<'a> {
    pub fn new(fault: &'a mut usize, resume: &'a mut usize) -> Self {
        Self {
            fault: *fault,
            resume: *resume,
            published_fault: fault,
            published_resume: resume,
        }
    }

    pub fn publish_fault(&mut self) {
        *self.published_fault = self.fault;
        #[cfg(feature = "profiling")]
        super::cold::event("run_frame_fault_pc_write");
    }
}
impl Drop for ProgramCounter<'_> {
    fn drop(&mut self) {
        self.publish_fault();
        *self.published_resume = self.resume;
        #[cfg(feature = "profiling")]
        super::cold::event("run_frame_resume_pc_write");
    }
}

#[cfg(test)]
mod tests {
    use super::ProgramCounter;

    #[test]
    fn local_pc_preserves_both_values_on_result_error_and_unwind() {
        let mut fault = 3;
        let mut resume = 4;
        let result: Result<(), ()> = (|| {
            let mut pc = ProgramCounter::new(&mut fault, &mut resume);
            pc.fault = 11;
            pc.resume = 12;
            Err(())
        })();
        assert!(result.is_err());
        assert_eq!((fault, resume), (11, 12));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut pc = ProgramCounter::new(&mut fault, &mut resume);
            pc.fault = 21;
            pc.resume = 22;
            panic!("local PC unwind probe");
        }));
        assert!(result.is_err());
        assert_eq!((fault, resume), (21, 22));
    }

    #[test]
    fn release_boundary_publishes_fault_before_leaving_resident_loop() {
        let mut fault = 3;
        let mut resume = 4;
        let mut pc = ProgramCounter::new(&mut fault, &mut resume);
        pc.fault = 11;
        pc.publish_fault();
        assert_eq!(*pc.published_fault, 11);
        // A later instruction is still materialized by the exit guard.
        pc.fault = 21;
        pc.resume = 22;
        drop(pc);
        assert_eq!((fault, resume), (21, 22));
    }
}
