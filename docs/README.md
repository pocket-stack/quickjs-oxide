# Documentation

## Current implementation

- [Architecture](architecture.md): package boundaries, semantic owners, current
  execution model and ownership rules.
- [Implementation status](status.md): implemented behavior and the pinned
  compatibility baseline.
- [Test262](test262.md), [parity contract](parity.md) and
  [registered deviations](deviations.md): acceptance and evidence.
- [Profiling and benchmarks](profiling.md): tools, measurement contracts and
  historical reports.
- [Browser playground](playground.md): build and host boundaries.

## Active stack VM redesign

As of 2026-09-15, stages S01–S08, S09 with its N1–N3 follow-ups and the new
S10–S12 (lazy frames, narrow state machine, property-read IC) are implemented
behind `--features stack-vm`. Performance exit conditions remain open: 25 of 58
fixed benchmarks are still slower than S0 (see the
[S10–S12 final report](performance/README.md)). The selected
representation is a stack VM with linear stack IR addressing issue #16's
native-stack, Number execution, call storage, local-update and PC-publication
problems.

| Document | Responsibility |
| --- | --- |
| [VM plan](primitive-vm-plan.md) | Goals, architecture decisions and scope |
| [Implementation design](primitive-vm-implementation-plan.md) | State ownership, proposed code structure and algorithms |
| [Commit plan](primitive-vm-commit-plan.md) | S01–S13 delivery order, checks and the S10–S12 ledger |
| [Lazy frames plan](primitive-vm-lazy-frames-plan.md) | S10–S12 design, audits and final acceptance record |
| [S14–S20 recovery plan](primitive-vm-s14-s20-recovery-plan.md) | Root causes of the remaining regressions and the staged fixes |
| [S14–S20 execution plan](primitive-vm-s14-s20-execution-plan.md) | Step-by-step work orders, tests and verification commands per stage |
| [Migration checklist](primitive-vm-migration.md) | Capability coverage, structure tasks and completion evidence |

## History

[Archived architecture and completed plans](archive/README.md) record earlier
source layouts and PR scopes. Reports under `docs/reports/` retain their
own measured baselines; their old priorities and validation results are not
current implementation instructions or fresh test results.
