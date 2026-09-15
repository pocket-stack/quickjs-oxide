# Owned execution spans

The span coverage and completion placement below describe the implemented
stages, not permanent restrictions on future optimizations. Extensions preserve
observable semantics, owner lifetimes and the validity of borrowed views.

S08's finite fusion is an immutable execution projection, not a new serialized
opcode set. `FunctionBytecodeData` owns the plan, and rooted execution snapshots
share it. The verified instruction array remains canonical, including all source
PCs, branch targets, handler addresses, resume addresses and legacy opcodes.
Publishing scans the existing instruction control contract once. A function with
no candidate has no plan allocation. A candidate-bearing function stores one byte
per canonical PC, with zero at ineligible PCs. Only span starts are tagged.

Supported spans:

| Span | Canonical instructions | Logical weight |
| --- | --- | --- |
| UpdateLocal prefix | GetLocal[/Check], Inc/Dec, SetLocal[/Check] | 3 |
| UpdateLocal postfix | GetLocal[/Check], PostInc/PostDec, PutLocal[/Check] | 3 |
| UpdateLocal discard | GetLocal[/Check], Inc/Dec, PutLocal[/Check] | 3 |
| UpdateLocal with discarded result | Prefix/postfix span followed by Drop | 4 |
| CompareBranch | Lt/Lte/Gt/Gte/Eq/Neq/StrictEq/StrictNeq, IfTrue/IfFalse | 2 |
| Primitive AddStore | Add, PutLocal[/Check] | 2 |
| Primitive AddStore with discarded result | Add, SetLocal[/Check], Drop | 3 |
| Borrowed LocalAdd | GetLocal[/Check] L, GetLocal[/Check] R, Add, PutLocal[/Check] L | 4 |
| Borrowed LocalAdd with discarded result | GetLocal[/Check] L, GetLocal[/Check] R, Add, SetLocal[/Check] L, Drop | 5 |
| Direct method call | GetField2, 0–7 literal/direct binding reads, matching CallMethod | 2–9 |

Any branch, catch or gosub target inside a proposed span rejects it. The next PC
after a control boundary is also an entry, covering structured gosub and
suspension resumption. Span patterns contain no explicit call, handler operation or
suspension. AddStore has a primitive operand guard and executes in the cold completion path. A target at a trailing Drop leaves that Drop outside the span.
Original instructions remain executable at all canonical addresses.

UpdateLocal requires a normal non-const local definition and a live Direct
Number binding. Captured, uninitialized, BigInt and coercible values retain the
original path. No result is changed before the Number guard succeeds. The same
`Number::update` kernel preserves overflow, signed zero, NaN and infinity; postfix
returns the original Number. Number owners require no retain, release drain or
GC. CompareBranch requires two Numbers and applies the original comparison before
testing branch polarity; negating `<` never becomes `>=`.

AddStore consumes operands already read at their canonical sites. Both must be
primitive, and the destination must still be a Direct initialized normal mutable
local; otherwise the unchanged conversion/store path runs. Shared primitive
addition executes after the Add PC is published and RunSlots has ended. A throw
leaves the binding unchanged. On success an Object/Symbol old binding retains
the store PC publication before replacement and last-owner release. Scalar,
String and BigInt storage cannot observe the runtime PC, so these paths commit
the final fault/resume pair without an extra store active publication. An
internal replacement error still publishes the canonical store site. A trailing
Drop only removes the redundant
assignment-result copy; its local owner remains live. This stage uses cold completion. String/BigInt allocation occurs outside
the RunSlots borrow; that boundary does not require leaving the run invocation. Captured, TDZ and const stores are excluded.

LocalAdd begins before either local is copied onto the operand stack. Publication
requires normal local definitions, a mutable left destination, the same left
index at the store, and no interior entry. At runtime both bindings must be
initialized Direct primitives, at least one must be String or BigInt, and the
window must have room for the two canonical pushes. Both value domains are
checked in left-to-right order. Objects, captured bindings, TDZ, insufficient
capacity and invalid domains decline without mutation. A malformed right index
also declines: the canonical second GetLocal diagnoses it after the first copy,
rather than reporting it prematurely at the span's entry PC.

After run releases RunSlots, `FrameTransaction::with_local_add_inputs` lends the
local values to the shared `numeric::add_primitives_ref` kernel. The immutable
borrows keep their local owners alive while primitive storage is allocated; no
callback can execute and borrowed references cannot escape the callback. This
removes both temporary operand roots. The ordinary owning addition entry uses
the same kernel. The Add PC is published before conversion/allocation. Successful
addition proves the old left binding is neither Object nor Symbol; its release
cannot observe the runtime PC, so one final fault/resume pair replaces redundant
store publication. An internal replacement error still publishes the store PC.
A primitive throw
keeps the old local and canonical Add fault site. On exhausted conversion
identity, the cold error branch reconstructs the two canonical operand copies
and reports the error at Add, matching the unfused stack and PC. Canonical
GetLocal instruction counts are recorded even when addition throws; successful
spans account for every remaining operation at its original logical depth.

Method spans admit PushI32, Undefined, Null, PushTrue, PushFalse and direct
GetLocal/GetLocalCheck/GetArg reads, with CallMethod's arity matching their count.
All binding reads are preflighted after lookup, before any argument copy. TDZ,
captured/private bindings and malformed indices decline to canonical evaluation.
Fallible owner copies preserve each argument PC and previously pushed arguments;
the error path publishes that PC after ending the slot borrow. Effectful
argument evaluation is never skipped or reordered. Property lookup remains a
live lookup at GetField2; getter/Proxy and unsupported read paths retain the
canonical fallback. An eligible own-data read can retain its receiver/callee,
materialize the literals/direct binding copies, publish CallMethod's canonical site, and enter
the existing call driver directly. Any transient native classification belongs
to that same retained callee and runtime; the stage stores it as a transient fact. Reuse across later lookups requires
validation of the corresponding property dependencies.
The call's existing realm, arity, budget, brand, error and cleanup rules remain
authoritative.

## Resident primitive arithmetic boundary

Same-frame completion in `ready::run` is different from remaining in one `run`
invocation: the former returns from run and rebuilds its transaction on reentry.
ConvertAdd/ConvertPlus and LocalAdd currently keep their existing completion
protocols; their ready-loop residency does not establish run residency. This
describes the implemented state, not a boundary constraint: the N1 pattern
below (end RunSlots, allocate inside the live FrameTransaction, reopen slots)
already satisfies the "allocation outside the RunSlots borrow" rule inside a
single run invocation. The S14–S20 recovery plan (S15) extends run residency to
Add's String/BigInt forms on this pattern; this section is rewritten when that
stage lands.

N1 defines a narrow boundary for eligible non-Object `NumericKind` arithmetic
after existing Number fast paths. Comparisons, abstract equality, Object inputs
and malformed slots keep canonical handling before any candidate consumption.
The resident path retains the original FrameTransaction, ends RunSlots, publishes
the arithmetic fault/active PC, takes owning operands in RHS-before-LHS order
through a short borrow, and calls the shared `primitive_output` outside RunSlots.
The same transaction reopens for pending output, previous before value for
postfix operations, and continues the current run only after successful commit.
It is a single opcode completion, not an additional fusion span.

Parsing, BigInt allocation, Symbol root release and error materialization must
remain outside RunSlots. A JS arithmetic error uses the executing frame realm
and the ordinary throw/unwind protocol; consumed inputs are never replayed.
Output errors retain canonical partial commits and fault/resume position.
The [numeric boundary audit](../performance/README.md)
records the ownership, domain, PC, callback, realm and error-channel constraints.
Source residency alone does not prove final throughput recovery.

## Current run PC representation

The completed PC candidate keeps fault and resume local during the exclusive
resident run borrow. A Drop guard materializes both Frame fields when run returns
normally, propagates an error, takes a cold exit, suspends, or unwinds in Rust.
The owner-release macro explicitly materializes fault before the existing
runtime active-PC publication and before releasing the displaced owner outside
RunSlots. The Runtime active stack is separate storage, not an alias of Frame.

UpdateLocal and CompareBranch retain their span-entry fault sites and existing
resume targets. AddStore and LocalAdd publish Add before conversion/allocation;
Object/Symbol release and store-error paths retain canonical store publication,
while unobservable primitive release permits a single final frame-PC commit.
Method spans distinguish property lookup, fallible argument copy and call sites.
Canonical source/debug tables are unchanged. Backtrace reads the Runtime active
stack; cold drivers and suspension inspect Frame only after guard writeback.

The [PC audit](../performance/README.md) enumerates all readers,
release boundaries, error materialization, overflow recovery, diagnostics and
unwind behavior. The actual Frame-write profiling counter moves with its write
sites; canonical logical-instruction counts do not change. Async CPU sampling
still has no arbitrary-instant exact JavaScript PC guarantee. This candidate is
covered by independent semantic checks; the requested final combined measurement
does not establish an isolated lazy-PC speedup.

## Observation points

| Observation | Span behavior |
| --- | --- |
| JS conversion, call or catchable throw | Number spans fall back before mutation. AddStore/LocalAdd can throw during shared primitive addition at the published Add PC; object conversion falls back. Method spans retain the canonical property and call observation sites. |
| Host call, allocation, GC or release drain | Number spans cannot perform these operations. Addition allocates only outside RunSlots and retains store publication for Object/Symbol release; unobservable primitive release needs no extra runtime publication. LocalAdd borrows existing roots through its transaction callback. Method spans enter the existing call driver. |
| Return, yield, await, handler entry or resume | Outside spans; canonical PCs and existing driver publication remain authoritative. |
| Backtrace and source location | Canonical source tables are unchanged. Number spans cannot throw internally; addition distinguishes addition from binding-release PCs, and method spans distinguish property lookup from call PCs. |
| Logical instruction profiling | Counts every canonical operation and its original intermediate stack depth. |
| Fuel, interrupt and single-step hooks | The current owned engine exposes none. Introducing such a hook must disable spans or first prove its budget covers the full logical weight; it must fall back canonically when observation falls inside a span. |
| Asynchronous CPU samples | Fault/resume remain local during run except explicit observation boundaries; driver Runtime publication does not promise an exact JavaScript PC at arbitrary sampling instants. |

For isolated A/B exports, replace `FusionPlan::update` with `None` to disable only
UpdateLocal, or `FusionPlan::compare_branch` with `false` to disable only
CompareBranch; replace `FusionPlan::add_store` with `false` to disable only the
cold AddStore completion. `FusionPlan::local_add_span` and `method_call`
returning `None` independently disable their spans. Preserve these changes only in immutable experiment trees, record
source hashes and binary identity, and retain canonical correctness checks. No
experimental feature flag belongs in production. Compile profiling measures
plan construction as `Fusion`; there is no additional relocation pass.
