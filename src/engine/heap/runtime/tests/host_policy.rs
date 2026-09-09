use super::*;

#[test]
fn can_block_policy_is_runtime_wide_and_isolated() {
    let runtime = Runtime::new();
    let clone = runtime.clone();
    let context = runtime.new_context();
    let other = Runtime::new();

    assert!(!runtime.can_block());
    assert!(!clone.can_block());
    assert!(!context.runtime().can_block());
    assert!(!other.can_block());

    clone.set_can_block(true);
    assert!(runtime.can_block());
    assert!(clone.can_block());
    assert!(context.runtime().can_block());
    assert!(!other.can_block());

    context.runtime().set_can_block(false);
    assert!(!runtime.can_block());
    assert!(!clone.can_block());
    assert!(!context.runtime().can_block());
}
