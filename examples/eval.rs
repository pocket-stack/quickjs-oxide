//! Minimal completion-value host.
//!
//! ```sh
//! cargo run --quiet --example eval -- '(function (a) { return a + 1; })(41)'
//! ```

use std::process::ExitCode;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{Runtime, RuntimeError, Value};

fn main() -> ExitCode {
    let mut arguments = std::env::args();
    let program = arguments.next().unwrap_or_else(|| "eval".to_owned());
    let Some(source) = arguments.next() else {
        eprintln!("usage: {program} JAVASCRIPT");
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: {program} JAVASCRIPT");
        return ExitCode::from(2);
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    match context.eval(&source) {
        Ok(value) => {
            println!("{}", completion_text(value));
            ExitCode::SUCCESS
        }
        Err(RuntimeError::Exception) => {
            let exception = context.take_exception().ok().flatten();
            eprintln!("uncaught JavaScript exception: {exception:?}");
            ExitCode::from(1)
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn completion_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => number_to_string(value),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "[object Object]".to_owned(),
        Value::Symbol(_) => "Symbol()".to_owned(),
    }
}
