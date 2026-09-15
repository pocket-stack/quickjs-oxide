//! The legacy interpreter consumes the same owned numeric conversion phases.
use super::{
    Completion, VmActivation, VmHost,
    numeric::operation::{NumericKind, NumericStep},
};
use crate::engine::api::Error;

impl VmActivation {
    pub(in crate::engine::vm) fn numeric_operation(
        &mut self,
        host: &mut impl VmHost,
        kind: NumericKind,
    ) -> Result<Option<Completion>, Error> {
        let (left, right) = if kind.unary() {
            (self.pop()?, None)
        } else {
            let (left, right) = self.pop_pair()?;
            (left, Some(right))
        };
        let mut step = NumericStep::start(kind, left, right)?;
        loop {
            step = match step {
                NumericStep::Complete { value, previous } => {
                    if let Some(previous) = previous {
                        self.stack.push(previous);
                    }
                    self.stack.push(value);
                    return Ok(None);
                }
                NumericStep::Throw(value) => return Ok(Some(Completion::Throw(value))),
                NumericStep::Primitive {
                    value,
                    hint,
                    resume,
                } => resume.resume(host.to_primitive(value, hint)?)?,
                NumericStep::HtmlDda { value, resume } => {
                    resume.html_dda(host.is_html_dda(&value)?)?
                }
            };
        }
    }
}
