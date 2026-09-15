//! Nested phase timing. These diagnostics are absent without `profiling`.
use super::{Collector, CostSnapshot, PhaseCost, current};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Instant;

#[derive(Clone, Copy)]
pub(crate) enum CompilePhase {
    Parse,
    Resolution,
    Lowering,
    Blocks,
    Fusion,
    Relocation,
    Verify,
    Publish,
}

impl CompilePhase {
    pub(super) fn cost_mut(self, costs: &mut CostSnapshot) -> &mut PhaseCost {
        match self {
            Self::Parse => &mut costs.parse,
            Self::Resolution => &mut costs.resolution,
            Self::Lowering => &mut costs.lowering,
            Self::Blocks => &mut costs.blocks,
            Self::Fusion => &mut costs.fusion,
            Self::Relocation => &mut costs.relocation,
            Self::Verify => &mut costs.verify,
            Self::Publish => &mut costs.publish,
        }
    }
}

type ActivePhase = (Weak<Collector>, Weak<Cell<u128>>);
thread_local! {
    static ACTIVE: RefCell<Vec<ActivePhase>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy)]
enum Phase {
    Compile(CompilePhase),
    Vm(&'static str),
}

struct Timing {
    collector: Rc<Collector>,
    started: Instant,
    phase: Phase,
    children: Rc<Cell<u128>>,
    parent: Option<Weak<Cell<u128>>>,
}

pub(crate) struct PhaseTimer(Option<Timing>);

impl PhaseTimer {
    pub(crate) fn start(phase: CompilePhase) -> Self {
        Self::start_phase(Phase::Compile(phase))
    }
    pub(crate) fn start_vm(name: &'static str) -> Self {
        Self::start_phase(Phase::Vm(name))
    }
    fn start_phase(phase: Phase) -> Self {
        Self(current().map(|collector| {
            let children = Rc::new(Cell::new(0));
            let parent = ACTIVE.with(|active| {
                let mut active = active.borrow_mut();
                active.retain(|(_, timer)| timer.strong_count() != 0);
                let owner = Rc::downgrade(&collector);
                let parent = active
                    .iter()
                    .rev()
                    .find_map(|(scope, timer)| scope.ptr_eq(&owner).then(|| timer.clone()));
                active.push((owner, Rc::downgrade(&children)));
                parent
            });
            Timing {
                collector,
                started: Instant::now(),
                phase,
                children,
                parent,
            }
        }))
    }
}

impl Timing {
    fn finish(&self, elapsed: u128) {
        let mut costs = self.collector.borrow_mut();
        let exclusive = elapsed.saturating_sub(self.children.get());
        let cost = match self.phase {
            Phase::Compile(phase) => phase.cost_mut(&mut costs),
            Phase::Vm(name) => {
                let phase = costs.vm_phases.entry(name).or_default();
                if phase.samples_ns.len() < 4096 {
                    phase.samples_ns.push([
                        elapsed.min(u128::from(u64::MAX)) as u64,
                        exclusive.min(u128::from(u64::MAX)) as u64,
                    ]);
                } else {
                    phase.omitted_samples = phase.omitted_samples.saturating_add(1);
                }
                &mut phase.cost
            }
        };
        cost.attempts = cost.attempts.saturating_add(1);
        cost.inclusive_ns = cost.inclusive_ns.saturating_add(elapsed);
        cost.exclusive_ns = cost.exclusive_ns.saturating_add(exclusive);
        if let Some(parent) = self.parent.as_ref().and_then(Weak::upgrade) {
            parent.set(parent.get().saturating_add(elapsed));
        }
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        if let Some(timing) = self.0.take() {
            timing.finish(timing.started.elapsed().as_nanos());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::api::profiling::CostProfile;

    #[test]
    fn child_time_is_subtracted_once_even_when_phases_recurse() {
        let profile = CostProfile::start();
        let mut parent = PhaseTimer::start(CompilePhase::Lowering);
        let mut child = PhaseTimer::start(CompilePhase::Lowering);
        let mut grandchild = PhaseTimer::start(CompilePhase::Verify);
        grandchild.0.take().unwrap().finish(3);
        child.0.take().unwrap().finish(8);
        parent.0.take().unwrap().finish(20);
        let costs = profile.snapshot();
        assert_eq!(costs.lowering.attempts, 2);
        assert_eq!(costs.lowering.inclusive_ns, 28);
        assert_eq!(costs.lowering.exclusive_ns, 17);
        assert_eq!(costs.verify.exclusive_ns, 3);
    }

    #[test]
    fn nested_collector_does_not_charge_its_phases_to_an_outer_collector() {
        let outer = CostProfile::start();
        let mut parent = PhaseTimer::start(CompilePhase::Parse);
        let inner = CostProfile::start();
        let mut child = PhaseTimer::start(CompilePhase::Verify);
        child.0.take().unwrap().finish(5);
        drop(inner);
        parent.0.take().unwrap().finish(10);
        let costs = outer.snapshot();
        assert_eq!(costs.parse.exclusive_ns, 10);
        assert_eq!(costs.verify.attempts, 0);
    }
    #[test]
    fn runtime_and_compile_nesting_share_one_exclusive_clock() {
        let profile = CostProfile::start();
        let mut parent = PhaseTimer::start_vm("freeze.encode");
        let mut child = PhaseTimer::start(CompilePhase::Verify);
        child.0.take().unwrap().finish(8);
        parent.0.take().unwrap().finish(20);
        let costs = profile.snapshot();
        assert_eq!(costs.vm_phases["freeze.encode"].samples_ns, vec![[20, 12]]);
        assert_eq!(costs.vm_phases["freeze.encode"].cost.exclusive_ns, 12);
        assert_eq!(costs.verify.exclusive_ns, 8);
    }
}
