//! Context-sensitive module-initializer flow validation.
//!
//! QuickJS lowers `finally` through shared `Gosub`/`Ret` bytecode. Keeping the
//! complete return-PC stack in a reachability key is exact but exponential for
//! nested shared subroutines. This verifier instead memoizes each subroutine by
//! `(entry PC, initializer ordinal)`. Normal `Ret` outcomes reconnect to the
//! concrete call site, while catch and iterator cleanup outcomes bubble to the
//! statically verified lower Gosub depth. The resulting analysis is polynomial
//! in the bytecode graph and never needs an arbitrary state budget.
//! `verify_parts` establishes the unique depth and unwind shape projected here;
//! any future opcode that observes or truncates either stack must be modeled in
//! both the shape builder and the summary analysis below.

use super::*;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::bytecode::Instruction;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Initializer {
    Declaration(u16),
    ImportCollision(u16),
}

fn initializer(instruction: &Instruction) -> Option<Initializer> {
    match instruction {
        Instruction::InitializeVarRef(index) => Some(Initializer::Declaration(*index)),
        Instruction::InitializeModuleImportCollision(index) => {
            Some(Initializer::ImportCollision(*index))
        }
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum UnwindFrame {
    Catch { target: usize, gosub_depth: usize },
    Iterator { gosub_depth: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ControlShape {
    gosub_depth: usize,
    regions: Vec<UnwindFrame>,
}

fn internal(message: &'static str) -> RuntimeError {
    RuntimeError::Engine(Error::internal(message))
}

fn target(target: u32, message: &'static str) -> Result<usize, RuntimeError> {
    usize::try_from(target).map_err(|_| internal(message))
}

fn fallthrough(pc: usize) -> Result<usize, RuntimeError> {
    pc.checked_add(1)
        .ok_or_else(|| internal("module evaluation fallthrough PC overflowed"))
}

fn enqueue_shape(
    shapes: &mut [Option<ControlShape>],
    worklist: &mut VecDeque<usize>,
    pc: usize,
    shape: ControlShape,
) -> Result<(), RuntimeError> {
    let slot = shapes
        .get_mut(pc)
        .ok_or_else(|| internal("module evaluation control flow escaped bytecode"))?;
    match slot {
        Some(previous) if previous != &shape => Err(internal(
            "module evaluation control flow has inconsistent unwind shape",
        )),
        Some(_) => Ok(()),
        None => {
            *slot = Some(shape);
            worklist.push_back(pc);
            Ok(())
        }
    }
}

fn build_control_shapes(
    code: &[Instruction],
    body: usize,
) -> Result<Vec<Option<ControlShape>>, RuntimeError> {
    let mut shapes = vec![None; code.len()];
    let mut worklist = VecDeque::new();
    enqueue_shape(
        &mut shapes,
        &mut worklist,
        body,
        ControlShape {
            gosub_depth: 0,
            regions: Vec::new(),
        },
    )?;

    while let Some(pc) = worklist.pop_front() {
        let mut shape = shapes[pc]
            .clone()
            .ok_or_else(|| internal("module evaluation control shape disappeared"))?;
        match &code[pc] {
            Instruction::Return
            | Instruction::ReturnDerived(_)
            | Instruction::Throw
            | Instruction::ThrowReadOnly(_)
            | Instruction::ThrowRedeclaration(_)
            | Instruction::ThrowDeleteSuper
            | Instruction::ThrowIteratorMissingThrow
            | Instruction::Ret => {}
            Instruction::Goto(destination) => {
                let destination = target(
                    *destination,
                    "module evaluation control-flow target overflowed",
                )?;
                enqueue_shape(&mut shapes, &mut worklist, destination, shape)?;
            }
            Instruction::IfFalse(destination) | Instruction::IfTrue(destination) => {
                let destination = target(
                    *destination,
                    "module evaluation control-flow target overflowed",
                )?;
                enqueue_shape(&mut shapes, &mut worklist, destination, shape.clone())?;
                enqueue_shape(&mut shapes, &mut worklist, fallthrough(pc)?, shape)?;
            }
            Instruction::Catch(handler) => {
                let handler = target(*handler, "module evaluation catch target overflowed")?;
                enqueue_shape(&mut shapes, &mut worklist, handler, shape.clone())?;
                shape.regions.push(UnwindFrame::Catch {
                    target: handler,
                    gosub_depth: shape.gosub_depth,
                });
                enqueue_shape(&mut shapes, &mut worklist, fallthrough(pc)?, shape)?;
            }
            Instruction::DropCatch => {
                let region = shape
                    .regions
                    .pop()
                    .ok_or_else(|| internal("module evaluation catch stack underflowed"))?;
                if !matches!(region, UnwindFrame::Catch { .. }) {
                    return Err(internal(
                        "module evaluation DropCatch targeted a non-catch region",
                    ));
                }
                enqueue_shape(&mut shapes, &mut worklist, fallthrough(pc)?, shape)?;
            }
            Instruction::NipCatch => {
                let region = shape
                    .regions
                    .pop()
                    .ok_or_else(|| internal("module evaluation catch stack underflowed"))?;
                let UnwindFrame::Catch { gosub_depth, .. } = region else {
                    return Err(internal(
                        "module evaluation NipCatch targeted a non-catch region",
                    ));
                };
                shape.gosub_depth = gosub_depth;
                enqueue_shape(&mut shapes, &mut worklist, fallthrough(pc)?, shape)?;
            }
            Instruction::Gosub(destination) => {
                let destination =
                    target(*destination, "module evaluation gosub target overflowed")?;
                let mut child = shape.clone();
                child.gosub_depth = child
                    .gosub_depth
                    .checked_add(1)
                    .ok_or_else(|| internal("module evaluation gosub depth overflowed"))?;
                enqueue_shape(&mut shapes, &mut worklist, destination, child)?;
                enqueue_shape(&mut shapes, &mut worklist, fallthrough(pc)?, shape)?;
            }
            Instruction::DropGosub => {
                shape.gosub_depth = shape
                    .gosub_depth
                    .checked_sub(1)
                    .ok_or_else(|| internal("module evaluation gosub stack underflowed"))?;
                enqueue_shape(&mut shapes, &mut worklist, fallthrough(pc)?, shape)?;
            }
            Instruction::ForOfStart | Instruction::ForAwaitOfStart => {
                shape.regions.push(UnwindFrame::Iterator {
                    gosub_depth: shape.gosub_depth,
                });
                enqueue_shape(&mut shapes, &mut worklist, fallthrough(pc)?, shape)?;
            }
            Instruction::IteratorClose
            | Instruction::IteratorClosePreserve
            | Instruction::IteratorDropPreserve
            | Instruction::IteratorDetachPreserve => {
                let region = shape
                    .regions
                    .pop()
                    .ok_or_else(|| internal("module evaluation iterator stack underflowed"))?;
                let UnwindFrame::Iterator { gosub_depth } = region else {
                    return Err(internal(
                        "module evaluation iterator cleanup targeted a non-iterator region",
                    ));
                };
                shape.gosub_depth = gosub_depth;
                enqueue_shape(&mut shapes, &mut worklist, fallthrough(pc)?, shape)?;
            }
            _ => enqueue_shape(&mut shapes, &mut worklist, fallthrough(pc)?, shape)?,
        }
    }
    Ok(shapes)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FlowPoint {
    pc: usize,
    next_initializer: usize,
}

#[derive(Clone, Debug, Default)]
struct FlowSummary {
    returns: HashSet<usize>,
    escapes: HashSet<FlowPoint>,
}

#[derive(Clone, Copy, Debug)]
struct PendingCall {
    child: FlowPoint,
    continuation: usize,
}

struct SummaryFrame {
    key: FlowPoint,
    level: usize,
    worklist: VecDeque<FlowPoint>,
    visited: HashSet<FlowPoint>,
    returns: HashSet<usize>,
    escapes: HashSet<FlowPoint>,
    pending: Option<PendingCall>,
}

impl SummaryFrame {
    fn new(key: FlowPoint, level: usize) -> Self {
        Self {
            key,
            level,
            worklist: VecDeque::from([key]),
            visited: HashSet::new(),
            returns: HashSet::new(),
            escapes: HashSet::new(),
            pending: None,
        }
    }

    fn finish(self) -> FlowSummary {
        FlowSummary {
            returns: self.returns,
            escapes: self.escapes,
        }
    }
}

fn shape_at(shapes: &[Option<ControlShape>], pc: usize) -> Result<&ControlShape, RuntimeError> {
    shapes
        .get(pc)
        .and_then(Option::as_ref)
        .ok_or_else(|| internal("module evaluation reached an unauthenticated control point"))
}

fn route(
    frame: &mut SummaryFrame,
    point: FlowPoint,
    shapes: &[Option<ControlShape>],
) -> Result<(), RuntimeError> {
    let destination_level = shape_at(shapes, point.pc)?.gosub_depth;
    if destination_level == frame.level {
        frame.worklist.push_back(point);
        Ok(())
    } else if destination_level < frame.level {
        frame.escapes.insert(point);
        Ok(())
    } else {
        Err(internal(
            "module evaluation entered a deeper Gosub without Gosub bytecode",
        ))
    }
}

fn route_exception(
    frame: &mut SummaryFrame,
    point: FlowPoint,
    shapes: &[Option<ControlShape>],
) -> Result<(), RuntimeError> {
    let Some(handler) = shape_at(shapes, point.pc)?
        .regions
        .iter()
        .rev()
        .find_map(|region| match region {
            UnwindFrame::Catch { target, .. } => Some(*target),
            UnwindFrame::Iterator { .. } => None,
        })
    else {
        return Ok(());
    };
    route(
        frame,
        FlowPoint {
            pc: handler,
            next_initializer: point.next_initializer,
        },
        shapes,
    )
}

fn apply_call_summary(
    frame: &mut SummaryFrame,
    continuation: usize,
    summary: &FlowSummary,
    shapes: &[Option<ControlShape>],
) -> Result<(), RuntimeError> {
    for next_initializer in &summary.returns {
        route(
            frame,
            FlowPoint {
                pc: continuation,
                next_initializer: *next_initializer,
            },
            shapes,
        )?;
    }
    for escape in &summary.escapes {
        route(frame, *escape, shapes)?;
    }
    Ok(())
}

// These operations cannot create a catchable ECMAScript abrupt completion.
// Allocation failure and violated publication invariants remain engine errors,
// not JavaScript throws which may enter an authored Catch handler.
fn may_throw_js(instruction: &Instruction) -> bool {
    !matches!(
        instruction,
        Instruction::Nop
            | Instruction::PushI32(_)
            | Instruction::PushAtomValueIndex(_)
            | Instruction::PushConst(_)
            | Instruction::FClosure(_)
            | Instruction::RegExp(_)
            | Instruction::Undefined
            | Instruction::Null
            | Instruction::PushFalse
            | Instruction::PushTrue
            | Instruction::PushThis
            | Instruction::PushActiveFunction
            | Instruction::PushHomeObject
            | Instruction::PushNewTarget
            | Instruction::Arguments(_)
            | Instruction::Rest(_)
            | Instruction::VariableEnvironment
            | Instruction::GetLocal(_)
            | Instruction::PutLocal(_)
            | Instruction::SetLocal(_)
            | Instruction::SetLocalUninitialized(_)
            | Instruction::InitializeLocal(_)
            | Instruction::GetArg(_)
            | Instruction::PutArg(_)
            | Instruction::SetArg(_)
            | Instruction::GetVarRef(_)
            | Instruction::PutVarRef(_)
            | Instruction::SetVarRef(_)
            | Instruction::InitializeVarRef(_)
            | Instruction::InitializeModuleImportCollision(_)
            | Instruction::CloseLocal(_)
            | Instruction::InitializePrivateName(_)
            | Instruction::ArrayFrom(_)
            | Instruction::Object
            | Instruction::Insert2
            | Instruction::Insert3
            | Instruction::Dup3
            | Instruction::Insert4
            | Instruction::Perm3
            | Instruction::Perm4
            | Instruction::Perm5
            | Instruction::Rot4Left
            | Instruction::Drop
            | Instruction::Nip
            | Instruction::Swap
            | Instruction::Dup
            | Instruction::Dup1
            | Instruction::Not
            | Instruction::TypeOf
            | Instruction::IsUndefinedOrNull
            | Instruction::StrictEq
            | Instruction::StrictNeq
            | Instruction::MarkSuperCall
            | Instruction::InitialYield
    )
}

fn flow_error() -> RuntimeError {
    internal("module lexical initializer is not a one-shot control-flow cut")
}

fn analyze(
    code: &[Instruction],
    shapes: &[Option<ControlShape>],
    expected: &[Initializer],
    body: usize,
) -> Result<(), RuntimeError> {
    let root = FlowPoint {
        pc: body,
        next_initializer: 0,
    };
    let mut summaries = HashMap::<FlowPoint, FlowSummary>::new();
    let mut active = HashSet::from([root]);
    let mut frames = vec![SummaryFrame::new(root, 0)];

    while !frames.is_empty() {
        if let Some(pending) = frames
            .last_mut()
            .expect("analysis frame remains present")
            .pending
            .take()
        {
            let summary = summaries
                .get(&pending.child)
                .cloned()
                .ok_or_else(|| internal("module Gosub summary disappeared"))?;
            apply_call_summary(
                frames.last_mut().expect("analysis frame remains present"),
                pending.continuation,
                &summary,
                shapes,
            )?;
            continue;
        }

        let point = frames
            .last_mut()
            .expect("analysis frame remains present")
            .worklist
            .pop_front();
        let Some(point) = point else {
            let frame = frames.pop().expect("analysis frame remains present");
            active.remove(&frame.key);
            summaries.insert(frame.key, frame.finish());
            continue;
        };
        if !frames
            .last_mut()
            .expect("analysis frame remains present")
            .visited
            .insert(point)
        {
            continue;
        }

        let instruction = code
            .get(point.pc)
            .ok_or_else(|| internal("module evaluation control flow escaped bytecode"))?;
        let mut next_initializer = point.next_initializer;
        if let Some(authored) = initializer(instruction) {
            if expected.get(next_initializer) != Some(&authored) {
                return Err(flow_error());
            }
            next_initializer += 1;
        }
        let after = FlowPoint {
            pc: point.pc,
            next_initializer,
        };
        let level = frames.last().expect("analysis frame remains present").level;

        match instruction {
            Instruction::Return => {
                if next_initializer != expected.len() {
                    return Err(flow_error());
                }
            }
            Instruction::ReturnDerived(_) => {
                if next_initializer != expected.len() {
                    return Err(flow_error());
                }
                route_exception(
                    frames.last_mut().expect("analysis frame remains present"),
                    after,
                    shapes,
                )?;
            }
            Instruction::Throw
            | Instruction::ThrowReadOnly(_)
            | Instruction::ThrowRedeclaration(_)
            | Instruction::ThrowDeleteSuper
            | Instruction::ThrowIteratorMissingThrow => {
                route_exception(
                    frames.last_mut().expect("analysis frame remains present"),
                    after,
                    shapes,
                )?;
            }
            Instruction::Goto(destination) => {
                let destination = target(
                    *destination,
                    "module evaluation control-flow target overflowed",
                )?;
                route(
                    frames.last_mut().expect("analysis frame remains present"),
                    FlowPoint {
                        pc: destination,
                        next_initializer,
                    },
                    shapes,
                )?;
            }
            Instruction::IfFalse(destination) | Instruction::IfTrue(destination) => {
                let destination = target(
                    *destination,
                    "module evaluation control-flow target overflowed",
                )?;
                let frame = frames.last_mut().expect("analysis frame remains present");
                route(
                    frame,
                    FlowPoint {
                        pc: destination,
                        next_initializer,
                    },
                    shapes,
                )?;
                route(
                    frame,
                    FlowPoint {
                        pc: fallthrough(point.pc)?,
                        next_initializer,
                    },
                    shapes,
                )?;
            }
            Instruction::Catch(_)
            | Instruction::DropCatch
            | Instruction::NipCatch
            | Instruction::DropGosub
            | Instruction::IteratorDropPreserve
            | Instruction::IteratorDetachPreserve => {
                route(
                    frames.last_mut().expect("analysis frame remains present"),
                    FlowPoint {
                        pc: fallthrough(point.pc)?,
                        next_initializer,
                    },
                    shapes,
                )?;
            }
            Instruction::Gosub(destination) => {
                let destination =
                    target(*destination, "module evaluation gosub target overflowed")?;
                let child = FlowPoint {
                    pc: destination,
                    next_initializer,
                };
                let continuation = fallthrough(point.pc)?;
                let child_level = shape_at(shapes, destination)?.gosub_depth;
                let expected_child_level = level
                    .checked_add(1)
                    .ok_or_else(|| internal("module Gosub depth overflowed"))?;
                if child_level != expected_child_level {
                    return Err(internal("module Gosub target has an invalid depth"));
                }
                if let Some(summary) = summaries.get(&child).cloned() {
                    apply_call_summary(
                        frames.last_mut().expect("analysis frame remains present"),
                        continuation,
                        &summary,
                        shapes,
                    )?;
                } else {
                    if !active.insert(child) {
                        return Err(internal("module Gosub summary recursed at one depth"));
                    }
                    frames
                        .last_mut()
                        .expect("analysis frame remains present")
                        .pending = Some(PendingCall {
                        child,
                        continuation,
                    });
                    frames.push(SummaryFrame::new(child, child_level));
                }
            }
            Instruction::Ret => {
                if level == 0 {
                    return Err(internal("module evaluation Ret escaped its Gosub"));
                }
                frames
                    .last_mut()
                    .expect("analysis frame remains present")
                    .returns
                    .insert(next_initializer);
            }
            Instruction::ForOfStart | Instruction::ForAwaitOfStart => {
                let frame = frames.last_mut().expect("analysis frame remains present");
                route_exception(frame, after, shapes)?;
                route(
                    frame,
                    FlowPoint {
                        pc: fallthrough(point.pc)?,
                        next_initializer,
                    },
                    shapes,
                )?;
            }
            Instruction::IteratorClose | Instruction::IteratorClosePreserve => {
                let frame = frames.last_mut().expect("analysis frame remains present");
                route_exception(frame, after, shapes)?;
                route(
                    frame,
                    FlowPoint {
                        pc: fallthrough(point.pc)?,
                        next_initializer,
                    },
                    shapes,
                )?;
            }
            _ => {
                let frame = frames.last_mut().expect("analysis frame remains present");
                if may_throw_js(instruction) {
                    route_exception(frame, after, shapes)?;
                }
                route(
                    frame,
                    FlowPoint {
                        pc: fallthrough(point.pc)?,
                        next_initializer,
                    },
                    shapes,
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn verify(module: &UnlinkedModule, body: usize) -> Result<(), RuntimeError> {
    let function = module.function();
    let descriptors = function.closure_variables();
    let mut expected = Vec::new();
    for index in module.declaration_order() {
        let descriptor = descriptors.get(usize::from(*index)).ok_or_else(|| {
            internal("module declaration order referenced a missing closure slot")
        })?;
        match descriptor.source {
            ClosureSource::ModuleDeclaration if descriptor.is_lexical => {
                expected.push(Initializer::Declaration(*index));
            }
            ClosureSource::ModuleImportCollision => {
                let declaration = module
                    .import_collisions()
                    .iter()
                    .find(|collision| collision.closure_index == *index)
                    .map(|collision| collision.declaration)
                    .ok_or_else(|| {
                        internal("module import collision has no declaration ledger entry")
                    })?;
                if declaration == ModuleImportCollisionDeclaration::Lexical {
                    expected.push(Initializer::ImportCollision(*index));
                }
            }
            _ => {}
        }
    }

    let code = function.code();
    let authored = code[body..]
        .iter()
        .filter_map(initializer)
        .collect::<Vec<_>>();
    if authored != expected {
        return Err(internal(
            "module lexical initializer order disagrees with declarations",
        ));
    }
    if expected.is_empty() {
        return Ok(());
    }

    let shapes = build_control_shapes(code, body)?;
    analyze(code, &shapes, &expected, body)
}
