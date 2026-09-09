//! Optional native host services for quickjs-oxide.
mod system;
pub use system::SystemHostServices;

// Cargo compiles a separate engine instance for its own unit tests. This
// opt-in bridge identifies the trait implemented by the native provider.
#[cfg(feature = "engine-test-support")]
pub mod engine_test {
    pub use quickjs_oxide::engine::api::HostServices;
}
