# Archived architecture and completed plans

Archived on 2026-09-12. These documents preserve prior decisions and recorded
implementation or validation states. Words such as “current”, “next” and
“must” inside them refer to their original stage, not to the active VM
redesign. In particular, old directory README rules and decisions to retain
recursive execution do not apply to the new work.

The current entries are [architecture](../architecture.md) and the
[stack VM plan](../primitive-vm-plan.md).

| Record | Historical scope |
| --- | --- |
| [Interpreter architecture research](interpreter-architecture.html) | Earlier alternatives and the Candidate A module decomposition |
| [Candidate A refactor record](candidate-a-refactor.md) | Prior package/source migrations and their validation receipts |
| [Data structure plan](data-structure-plan.md) | PR #17 implementation and acceptance record |
| [Published execution plan](published-execution-plan.md) | PR #18 scope and implementation record, based on PR #17 `b11f2be` |
| [Published execution contracts](published-execution-contracts.md) | Contracts documented for that PR #18 execution path |
| [Ordinary property plan](ordinary-property-plan.md) | PR #19 property-kernel work; recorded source check at `fec7519` |

Navigation has been updated and mandatory directory README instructions
removed. Recorded commits, measurements and verification results keep their
original meaning. Archiving is not a new PR-status check, benchmark run or conformance
receipt. Linked source paths describe their recorded implementations and
may move as the current plan is implemented.
