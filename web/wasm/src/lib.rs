use js_sys::{Date, Math, Object, Reflect};
use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{
    Context, HostServices, JsString, PropertyKey, QUICKJS_COMPAT_VERSION, QUICKJS_OXIDE_VERSION,
    Runtime, RuntimeError, Value,
};
use wasm_bindgen::prelude::*;

const PLAYGROUND_FILENAME: &str = "<playground>";
const WEB_CAN_BLOCK: bool = false;
const TWO_TO_THE_32: f64 = 4_294_967_296.0;

const BUILD_COMMIT: &str = match option_env!("QUICKJS_OXIDE_COMMIT") {
    Some(commit) => commit,
    None => "local",
};

#[derive(Debug, Default)]
struct WebHostServices;

impl HostServices for WebHostServices {
    fn now_millis(&self) -> i64 {
        Date::now() as i64
    }

    fn timezone_offset_minutes(&self, epoch_millis: i64) -> i32 {
        let date = Date::new(&JsValue::from_f64(epoch_millis as f64));
        let offset = date.get_timezone_offset();
        if offset.is_finite() { offset as i32 } else { 0 }
    }

    fn random_seed(&self) -> u64 {
        let high = (Math::random() * TWO_TO_THE_32) as u64;
        let low = (Math::random() * TWO_TO_THE_32) as u64;
        (high << 32) | low
    }
}

struct EvalResult {
    ok: bool,
    kind: &'static str,
    text: String,
}

impl EvalResult {
    fn success(runtime: &Runtime, value: &Value) -> Self {
        let (kind, text) = value_text(runtime, value);
        Self {
            ok: true,
            kind,
            text,
        }
    }

    fn engine_error(error: &RuntimeError) -> Self {
        Self {
            ok: false,
            kind: "engine-error",
            text: error.to_string(),
        }
    }

    fn exception(runtime: &Runtime, context: &mut Context) -> Self {
        match context.take_exception() {
            Ok(Some(value)) => Self {
                ok: false,
                kind: "exception",
                text: exception_text(runtime, &value),
            },
            Ok(None) => Self {
                ok: false,
                kind: "exception",
                text: "JavaScript exception".to_owned(),
            },
            Err(error) => Self::engine_error(&error),
        }
    }

    fn into_js(self) -> JsValue {
        let object = Object::new();
        set_result_field(&object, "ok", &JsValue::from_bool(self.ok));
        set_result_field(&object, "kind", &JsValue::from_str(self.kind));
        set_result_field(&object, "text", &JsValue::from_str(&self.text));
        object.into()
    }
}

/// Evaluate one isolated script with quickjs-oxide's Rust compiler and runtime.
///
/// A fresh runtime and realm are used for every call. The completion is returned
/// as a plain JavaScript object with stable `ok`, `kind`, and `text` fields.
#[wasm_bindgen]
pub fn evaluate(source: &str) -> JsValue {
    evaluate_with_engine(source).into_js()
}

/// Return provenance and host-policy metadata for this exact WebAssembly build.
#[wasm_bindgen]
pub fn engine_metadata() -> JsValue {
    let object = Object::new();
    set_result_field(&object, "engine", &JsValue::from_str("quickjs-oxide"));
    set_result_field(
        &object,
        "crateVersion",
        &JsValue::from_str(QUICKJS_OXIDE_VERSION),
    );
    set_result_field(
        &object,
        "quickjsTarget",
        &JsValue::from_str(&format!("QuickJS {QUICKJS_COMPAT_VERSION}")),
    );
    set_result_field(&object, "buildCommit", &JsValue::from_str(BUILD_COMMIT));
    set_result_field(&object, "canBlock", &JsValue::from_bool(WEB_CAN_BLOCK));
    object.into()
}

fn evaluate_with_engine(source: &str) -> EvalResult {
    let runtime = Runtime::new_with_host_services(WebHostServices);
    runtime.set_can_block(WEB_CAN_BLOCK);
    let mut context = runtime.new_context();
    let value = match context.eval_with_filename(source, PLAYGROUND_FILENAME) {
        Ok(value) => value,
        Err(RuntimeError::Exception) => return EvalResult::exception(&runtime, &mut context),
        Err(error) => return EvalResult::engine_error(&error),
    };

    loop {
        match runtime.execute_pending_job() {
            Ok(true) => {}
            Ok(false) => break,
            Err(RuntimeError::Exception) => {
                return EvalResult::exception(&runtime, &mut context);
            }
            Err(error) => return EvalResult::engine_error(&error),
        }
    }

    EvalResult::success(&runtime, &value)
}

fn value_text(runtime: &Runtime, value: &Value) -> (&'static str, String) {
    match value {
        Value::Undefined => ("undefined", "undefined".to_owned()),
        Value::Null => ("null", "null".to_owned()),
        Value::Bool(value) => ("boolean", value.to_string()),
        Value::Int(value) => ("number", value.to_string()),
        Value::Float(value) => ("number", number_to_string(*value)),
        Value::BigInt(value) => ("bigint", format!("{value}n")),
        Value::String(value) => ("string", value.to_utf8_lossy()),
        Value::Object(_) => ("object", "[object Object]".to_owned()),
        Value::Symbol(symbol) => {
            let description = runtime
                .property_key_to_js_string(&PropertyKey::from(symbol))
                .map_or_else(|_| String::new(), |value| value.to_utf8_lossy());
            ("symbol", format!("Symbol({description})"))
        }
    }
}

fn exception_text(runtime: &Runtime, value: &Value) -> String {
    if let Value::Object(object) = value {
        if runtime.is_error_object(object).unwrap_or(false) {
            let name =
                diagnostic_property(runtime, object, "name").unwrap_or_else(|| "Error".to_owned());
            let message = diagnostic_property(runtime, object, "message");
            return match message {
                Some(message) if !message.is_empty() => format!("{name}: {message}"),
                Some(_) | None => name,
            };
        }
    }

    value_text(runtime, value).1
}

fn diagnostic_property(
    runtime: &Runtime,
    object: &quickjs_oxide::ObjectRef,
    name: &str,
) -> Option<String> {
    let key = runtime.intern_property_key(name).ok()?;
    runtime
        .raw_string_property_for_diagnostics(object, &key)
        .ok()
        .flatten()
        .map(|value| diagnostic_c_string(&value))
}

fn diagnostic_c_string(value: &JsString) -> String {
    char::decode_utf16(value.utf16_units().take_while(|unit| *unit != 0))
        .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

fn set_result_field(object: &Object, key: &str, value: &JsValue) {
    Reflect::set(object, &JsValue::from_str(key), value)
        .expect("setting a fresh result object's data property must succeed");
}
