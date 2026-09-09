//! Browser implementation of engine host capabilities.
use js_sys::{Date, Math};
use quickjs_oxide::engine::api::HostServices;
use wasm_bindgen::JsValue;

const TWO_TO_THE_32: f64 = 4_294_967_296.0;

#[derive(Debug, Default)]
pub struct WebHostServices;

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
