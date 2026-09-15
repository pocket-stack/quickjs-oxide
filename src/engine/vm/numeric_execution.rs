use super::*;

/// Number-tag updates cannot call JavaScript and preserve an existing Float
/// tag, including signed zero. Int32 overflow promotes exactly once to Float64.
fn updated_number(value: &Value, increment: bool) -> Option<Value> {
    match value {
        Value::Int(old) => {
            let next = if increment {
                old.checked_add(1)
            } else {
                old.checked_sub(1)
            };
            Some(next.map_or_else(
                || Value::Float(f64::from(*old) + if increment { 1.0 } else { -1.0 }),
                Value::Int,
            ))
        }
        Value::Float(old) => Some(Value::Float(if increment {
            *old + 1.0
        } else {
            *old - 1.0
        })),
        _ => None,
    }
}

impl VmActivation {
    pub(in crate::engine::vm) fn neg(
        &mut self,
        host: &mut impl VmHost,
    ) -> Result<OperationOutcome<()>, Error> {
        let operand = match to_primitive(host, self.pop()?, ToPrimitiveHint::Number)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        match operand {
            Value::BigInt(value) => self
                .stack
                .push(Value::BigInt(value.neg().map_err(bigint_error)?)),
            // QuickJS uses __JS_NewFloat64 for a Float64 operand, retaining
            // the Float tag even when the result has an integral value.
            Value::Float(value) => self.stack.push(Value::Float(-value)),
            value => self.stack.push(Value::number(-value.to_number()?)),
        }
        Ok(OperationOutcome::Value(()))
    }

    pub(in crate::engine::vm) fn unary_plus(
        &mut self,
        host: &mut impl VmHost,
    ) -> Result<OperationOutcome<()>, Error> {
        let operand = match to_primitive(host, self.pop()?, ToPrimitiveHint::Number)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        if matches!(operand, Value::BigInt(_)) {
            return Err(Error::new(ErrorKind::Type, "bigint argument with unary +"));
        }
        match operand {
            // OP_plus is a no-op for both native numeric tags. In particular,
            // an integral Float64 must not be compacted to Int32.
            Value::Int(value) => self.stack.push(Value::Int(value)),
            Value::Float(value) => self.stack.push(Value::Float(value)),
            value => self.stack.push(Value::number(value.to_number()?)),
        }
        Ok(OperationOutcome::Value(()))
    }

    /// QuickJS `js_unary_arith_slow` / `js_post_inc_slow`. Postfix updates
    /// retain the converted numeric value, not the original string or object.
    pub(in crate::engine::vm) fn update_numeric(
        &mut self,
        host: &mut impl VmHost,
        increment: bool,
        postfix: bool,
    ) -> Result<OperationOutcome<()>, Error> {
        if let Some(top) = self.stack.last_mut()
            && let Some(next) = updated_number(top, increment)
        {
            if postfix {
                self.stack.push(next);
            } else {
                *top = next;
            }
            return Ok(OperationOutcome::Value(()));
        }
        let operand = match to_primitive(host, self.pop()?, ToPrimitiveHint::Number)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        if let Some(next) = updated_number(&operand, increment) {
            if postfix {
                self.stack.push(operand);
            }
            self.stack.push(next);
            return Ok(OperationOutcome::Value(()));
        }
        match operand {
            Value::BigInt(old) => {
                let one = JsBigInt::from(1_i32);
                let new = if increment {
                    old.add(&one)
                } else {
                    old.update_decrement()
                }
                .map_err(bigint_error)?;
                if postfix {
                    self.stack.push(Value::BigInt(old));
                }
                self.stack.push(Value::BigInt(new));
            }
            value => {
                let old = value.to_number()?;
                let new = if increment { old + 1.0 } else { old - 1.0 };
                if postfix {
                    self.stack.push(Value::number(old));
                }
                self.stack.push(Value::number(new));
            }
        }
        Ok(OperationOutcome::Value(()))
    }

    pub(in crate::engine::vm) fn bit_not(
        &mut self,
        host: &mut impl VmHost,
    ) -> Result<OperationOutcome<()>, Error> {
        let operand = match to_numeric(host, self.pop()?)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        match operand {
            NumericValue::BigInt(value) => self
                .stack
                .push(Value::BigInt(value.bit_not().map_err(bigint_error)?)),
            NumericValue::Number(value) => self.stack.push(Value::Int(!number_to_int32(value))),
        }
        Ok(OperationOutcome::Value(()))
    }

    pub(in crate::engine::vm) fn unsigned_shift_right(
        &mut self,
        host: &mut impl VmHost,
    ) -> Result<OperationOutcome<()>, Error> {
        let (left, right) = self.pop_pair()?;
        let left = match to_numeric(host, left)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let right = match to_numeric(host, right)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let (NumericValue::Number(left), NumericValue::Number(right)) = (left, right) else {
            return Err(Error::new(
                ErrorKind::Type,
                "bigint operands are forbidden for >>>",
            ));
        };
        let result = number_to_uint32(left) >> (number_to_uint32(right) & 0x1f);
        self.stack.push(Value::number(f64::from(result)));
        Ok(OperationOutcome::Value(()))
    }

    pub(in crate::engine::vm) fn binary_numeric(
        &mut self,
        host: &mut impl VmHost,
        number_operation: impl FnOnce(f64, f64) -> f64,
        bigint_operation: impl FnOnce(&JsBigInt, &JsBigInt) -> Result<JsBigInt, BigIntError>,
    ) -> Result<OperationOutcome<()>, Error> {
        let (left, right) = self.pop_pair()?;
        let left = match to_numeric(host, left)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let right = match to_numeric(host, right)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        match (left, right) {
            (NumericValue::BigInt(left), NumericValue::BigInt(right)) => {
                self.stack.push(Value::BigInt(
                    bigint_operation(&left, &right).map_err(bigint_error)?,
                ));
            }
            (NumericValue::BigInt(_), NumericValue::Number(_))
            | (NumericValue::Number(_), NumericValue::BigInt(_)) => {
                return Err(mixed_numeric_type_error());
            }
            (NumericValue::Number(left), NumericValue::Number(right)) => self
                .stack
                .push(Value::number(number_operation(left, right))),
        }
        Ok(OperationOutcome::Value(()))
    }

    pub(in crate::engine::vm) fn compare(
        &mut self,
        host: &mut impl VmHost,
        operation: impl FnOnce(std::cmp::Ordering) -> bool,
    ) -> Result<OperationOutcome<()>, Error> {
        let (left, right) = self.pop_pair()?;
        let left = match to_primitive(host, left, ToPrimitiveHint::Number)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let right = match to_primitive(host, right, ToPrimitiveHint::Number)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let ordering = match (&left, &right) {
            (Value::String(left), Value::String(right)) => {
                Some(left.utf16_units().cmp(right.utf16_units()))
            }
            (Value::BigInt(left), Value::BigInt(right)) => Some(left.cmp(right)),
            (Value::BigInt(left), Value::String(right)) => {
                string_to_bigint(right).map(|right| left.cmp(&right))
            }
            (Value::String(left), Value::BigInt(right)) => {
                string_to_bigint(left).map(|left| left.cmp(right))
            }
            _ => {
                let left = to_numeric_primitive(left)?;
                let right = to_numeric_primitive(right)?;
                match (&left, &right) {
                    (NumericValue::BigInt(left), NumericValue::BigInt(right)) => {
                        Some(left.cmp(right))
                    }
                    (NumericValue::BigInt(left), NumericValue::Number(right)) => {
                        compare_bigint_number(left, *right)
                    }
                    (NumericValue::Number(left), NumericValue::BigInt(right)) => {
                        compare_bigint_number(right, *left).map(std::cmp::Ordering::reverse)
                    }
                    (NumericValue::Number(left), NumericValue::Number(right)) => {
                        left.partial_cmp(right)
                    }
                }
            }
        };
        self.stack
            .push(Value::Bool(ordering.is_some_and(operation)));
        Ok(OperationOutcome::Value(()))
    }

    pub(in crate::engine::vm) fn add(
        &mut self,
        host: &mut impl VmHost,
    ) -> Result<OperationOutcome<()>, Error> {
        let (left, right) = self.pop_pair()?;
        let left = match to_primitive(host, left, ToPrimitiveHint::Default)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let right = match to_primitive(host, right, ToPrimitiveHint::Default)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
            let left = match left {
                Value::String(value) => value,
                value => value.to_js_string()?,
            };
            let right = match right {
                Value::String(value) => value,
                value => value.to_js_string()?,
            };
            self.stack
                .push(Value::String(left.try_concat(&right).map_err(Error::from)?));
        } else {
            let left = to_numeric_primitive(left)?;
            let right = to_numeric_primitive(right)?;
            match (left, right) {
                (NumericValue::BigInt(left), NumericValue::BigInt(right)) => self
                    .stack
                    .push(Value::BigInt(left.add(&right).map_err(bigint_error)?)),
                (NumericValue::BigInt(_), NumericValue::Number(_))
                | (NumericValue::Number(_), NumericValue::BigInt(_)) => {
                    return Err(mixed_numeric_type_error());
                }
                (NumericValue::Number(left), NumericValue::Number(right)) => {
                    self.stack.push(Value::number(left + right));
                }
            }
        }
        Ok(OperationOutcome::Value(()))
    }
}
