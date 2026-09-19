//! Direct arguments shared by the canonical linked read and resident IC span.
use super::bindings::FrameBinding;
use super::stack::{RunSlots, copy_value};
use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::code::bytecode::Instruction;
use crate::engine::value::JsValue;

pub(super) fn available(slots: &RunSlots<'_>, instructions: &[Instruction]) -> bool {
    instructions.iter().all(|instruction| match instruction {
        Instruction::GetLocal(index) | Instruction::GetLocalCheck(index) => {
            matches!(slots.local(*index), Ok(FrameBinding::Direct(_)))
        }
        Instruction::GetArg(index) => {
            matches!(slots.parameter(*index), Ok(FrameBinding::Direct(_)))
        }
        _ => true,
    })
}

pub(super) fn argument(
    runtime: &Runtime,
    slots: &RunSlots<'_>,
    instruction: &Instruction,
) -> Result<JsValue, Error> {
    Ok(match instruction {
        Instruction::GetLocal(index) | Instruction::GetLocalCheck(index) => {
            let FrameBinding::Direct(value) = slots.local(*index)? else {
                unreachable!("preflighted direct method argument")
            };
            copy_value(runtime, value)?
        }
        Instruction::GetArg(index) => {
            let FrameBinding::Direct(value) = slots.parameter(*index)? else {
                unreachable!("preflighted direct method parameter")
            };
            copy_value(runtime, value)?
        }
        Instruction::PushI32(value) => JsValue::Int(*value),
        Instruction::Undefined => JsValue::Undefined,
        Instruction::Null => JsValue::Null,
        Instruction::PushTrue => JsValue::Bool(true),
        Instruction::PushFalse => JsValue::Bool(false),
        _ => unreachable!("published method call span"),
    })
}
