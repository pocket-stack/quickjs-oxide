use crate::engine::api::error::ErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::Atom;
use crate::engine::heap::ObjectPayload;
use crate::engine::object::access::raw_string_property_one_level;

use crate::engine::object::{DescriptorField, ObjectRef, OrdinaryPropertyDescriptor};
use crate::engine::value::{JsString, JsStringBuilder, JsStringError, Value};
use crate::engine::vm::BytecodePc;
use crate::engine::vm::frames::{ActiveFrameKind, ExplicitBacktraceLocation};

impl Runtime {
    /// QuickJS `is_backtrace_needed` plus `build_backtrace`.
    ///
    /// Only real Error-class objects without any own `stack` property are
    /// eligible. Function names are read from raw ordinary data slots so this
    /// path never invokes user code while an exception is already in flight.
    pub(crate) fn ensure_error_backtrace(
        &self,
        value: &Value,
        skip_first_frame: bool,
        explicit_location: Option<ExplicitBacktraceLocation>,
    ) -> Result<(), RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(());
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("backtrace Error object"));
        }

        let stack_key = self.intern_property_key("stack")?;
        let needs_backtrace = {
            let state = self.0.state.borrow();
            let data = state.heap.object(object.object_id())?;
            if !matches!(data.payload, ObjectPayload::Error) {
                false
            } else {
                state
                    .heap
                    .shape(data.shape)?
                    .find(stack_key.atom())
                    .is_none()
            }
        };
        if !needs_backtrace {
            return Ok(());
        }

        let name_key = self.intern_property_key("name")?;
        let stack = match self.build_backtrace_string(
            name_key.atom(),
            skip_first_frame,
            explicit_location.as_ref(),
        ) {
            Ok(stack) => stack,
            Err(RuntimeError::Engine(error))
                if error.kind() == ErrorKind::JsInternal
                    && error.message() == "string too long" =>
            {
                // QuickJS's void build_backtrace helper must not replace the
                // Error already being materialized when its stack text cannot
                // become an ECMAScript String.
                return Ok(());
            }
            Err(error) => return Err(error),
        };

        // Parse errors add SpiderMonkey-compatible metadata before `stack`,
        // exactly as QuickJS does. Rejection (for example after
        // preventExtensions) is intentionally silent: build_backtrace must
        // not replace the original JavaScript completion.
        if let Some(location) = explicit_location {
            let Some((line, column)) = location.position.one_based() else {
                return Err(RuntimeError::Invariant(
                    "backtrace location cannot be represented one-based",
                ));
            };
            let line = i32::try_from(line).map_err(|_| {
                RuntimeError::Invariant("backtrace line does not fit an ECMAScript Int32")
            })?;
            let column = i32::try_from(column).map_err(|_| {
                RuntimeError::Invariant("backtrace column does not fit an ECMAScript Int32")
            })?;
            for (name, property_value) in [
                ("fileName", Value::String(location.filename)),
                ("lineNumber", Value::Int(line)),
                ("columnNumber", Value::Int(column)),
            ] {
                if !self.define_backtrace_property(object, name, property_value)? {
                    return Ok(());
                }
            }
        }

        let _ = self.define_backtrace_property(object, "stack", Value::String(stack))?;
        Ok(())
    }

    pub(crate) fn define_backtrace_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: Value,
    ) -> Result<bool, RuntimeError> {
        let key = self.intern_property_key(name)?;
        self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
    }

    pub(crate) fn build_backtrace_string(
        &self,
        name_atom: Atom,
        mut skip_first_frame: bool,
        explicit_location: Option<&ExplicitBacktraceLocation>,
    ) -> Result<JsString, RuntimeError> {
        let state = self.0.state.borrow();
        let mut output = JsStringBuilder::new(0);

        if let Some(location) = explicit_location {
            let (line, column) = location
                .position
                .one_based()
                .ok_or(RuntimeError::Invariant(
                    "backtrace location cannot be represented one-based",
                ))?;
            append_backtrace_ascii(&mut output, "    at ")?;
            append_backtrace_string(&mut output, &location.filename)?;
            append_backtrace_ascii(&mut output, ":")?;
            append_backtrace_ascii(&mut output, &line.to_string())?;
            append_backtrace_ascii(&mut output, ":")?;
            append_backtrace_ascii(&mut output, &column.to_string())?;
            append_backtrace_ascii(&mut output, "\n")?;
        }

        for frame in state.active_frames.iter().rev() {
            if frame.flags.backtrace_barrier {
                break;
            }
            if frame.flags.backtrace_hidden {
                continue;
            }
            if skip_first_frame {
                skip_first_frame = false;
                continue;
            }

            let name = raw_string_property_one_level(&state, frame.function, name_atom)?
                .map(truncate_backtrace_c_string)
                .transpose()?
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| JsString::from_static("<anonymous>"));
            append_backtrace_ascii(&mut output, "    at ")?;
            append_backtrace_string(&mut output, &name)?;

            match frame.kind {
                ActiveFrameKind::Native { .. } => {
                    append_backtrace_ascii(&mut output, " (native)")?;
                }
                ActiveFrameKind::Bytecode { bytecode, pc } => {
                    let bytecode = state.heap.function_bytecode(bytecode)?;
                    if let Some(debug) = &bytecode.debug {
                        let filename = state.atoms.to_js_string(debug.filename)?;
                        append_backtrace_ascii(&mut output, " (")?;
                        append_backtrace_string(&mut output, &filename)?;
                        if let Some(table) = &debug.pc2line {
                            let pc = pc
                                .map(BytecodePc::index)
                                .map(u32::try_from)
                                .transpose()
                                .map_err(|_| {
                                    RuntimeError::Invariant(
                                        "active bytecode PC does not fit debug metadata",
                                    )
                                })?;
                            let (line, column) =
                                table.lookup(pc).one_based().ok_or(RuntimeError::Invariant(
                                    "bytecode debug position cannot be represented one-based",
                                ))?;
                            append_backtrace_ascii(&mut output, ":")?;
                            append_backtrace_ascii(&mut output, &line.to_string())?;
                            append_backtrace_ascii(&mut output, ":")?;
                            append_backtrace_ascii(&mut output, &column.to_string())?;
                        }
                        append_backtrace_ascii(&mut output, ")")?;
                    }
                }
            }
            append_backtrace_ascii(&mut output, "\n")?;
        }

        Ok(output.finish()?)
    }
}

pub(crate) fn append_backtrace_ascii(
    output: &mut JsStringBuilder,
    value: &str,
) -> Result<(), JsStringError> {
    output.push_utf8(value)
}

pub(crate) fn append_backtrace_string(
    output: &mut JsStringBuilder,
    value: &JsString,
) -> Result<(), JsStringError> {
    output.push_js_string(value)
}

pub(crate) fn truncate_backtrace_c_string(value: JsString) -> Result<JsString, RuntimeError> {
    if !value.utf16_units().any(|unit| unit == 0) {
        return Ok(value);
    }
    let prefix = value.utf16_units().take_while(|unit| *unit != 0);
    Ok(JsString::try_from_utf16(prefix)?)
}
