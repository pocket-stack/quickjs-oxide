use super::*;

fn sum(values: &[f64]) -> f64 {
    let mut sum = SumPrecise::new();
    for value in values {
        sum.add(*value);
    }
    sum.result()
}

#[test]
fn min_max_preserve_pinned_signed_zero_rules() {
    assert_eq!(quickjs_min(0.0, -0.0).to_bits(), (-0.0_f64).to_bits());
    assert_eq!(quickjs_min(-0.0, 0.0).to_bits(), (-0.0_f64).to_bits());
    assert_eq!(quickjs_max(0.0, -0.0).to_bits(), 0.0_f64.to_bits());
    assert_eq!(quickjs_max(-0.0, -0.0).to_bits(), (-0.0_f64).to_bits());
}

#[test]
fn round_uses_ecmascript_ties_toward_positive_infinity() {
    for (input, expected) in [
        (-1.5, -1.0),
        (-0.5, -0.0),
        (-0.1, -0.0),
        (0.1, 0.0),
        (0.5, 1.0),
        (1.5, 2.0),
        (4_503_599_627_370_495.5, 4_503_599_627_370_496.0),
    ] as [(f64, f64); 7]
    {
        assert_eq!(
            quickjs_round(input).to_bits(),
            expected.to_bits(),
            "{input}"
        );
    }
    for value in [f64::NEG_INFINITY, f64::INFINITY, -0.0, 0.0] {
        assert_eq!(quickjs_round(value).to_bits(), value.to_bits());
    }
    assert!(quickjs_round(f64::NAN).is_nan());
}

#[test]
fn power_keeps_quickjs_non_ieee_infinite_exponent_case() {
    assert!(crate::number::pow(1.0, f64::INFINITY).is_nan());
    assert!(crate::number::pow(-1.0, f64::NEG_INFINITY).is_nan());
    assert_eq!(crate::number::pow(-0.0, -3.0), f64::NEG_INFINITY);
    assert_eq!(
        crate::number::pow(-0.0, 3.0).to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(crate::number::pow(f64::NAN, 0.0), 1.0);
}

#[test]
fn binary16_rounding_matches_quickjs_thresholds_and_ties() {
    for value in [f64::NAN, f64::NEG_INFINITY, f64::INFINITY, -0.0, 0.0] {
        let rounded = quickjs_f16round(value);
        if value.is_nan() {
            assert!(rounded.is_nan());
        } else {
            assert_eq!(rounded.to_bits(), value.to_bits());
        }
    }
    assert_eq!(quickjs_f16round(65_519.0), 65_504.0);
    assert_eq!(quickjs_f16round(65_520.0), f64::INFINITY);
    let minimum = 2.0_f64.powi(-24);
    assert_eq!(quickjs_f16round(minimum), minimum);
    assert_eq!(quickjs_f16round(minimum / 2.0).to_bits(), 0.0_f64.to_bits());
    assert_eq!(
        quickjs_f16round(-minimum / 2.0).to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(quickjs_f16round(1.337), 1.336_914_062_5);
}

#[test]
fn sum_precise_preserves_zero_infinity_and_exact_rounding_states() {
    assert_eq!(sum(&[]).to_bits(), (-0.0_f64).to_bits());
    assert_eq!(sum(&[-0.0, -0.0]).to_bits(), (-0.0_f64).to_bits());
    assert_eq!(sum(&[-0.0, 0.0]).to_bits(), 0.0_f64.to_bits());
    assert_eq!(sum(&[1.0, -1.0]).to_bits(), 0.0_f64.to_bits());
    assert_eq!(sum(&[f64::INFINITY]), f64::INFINITY);
    assert_eq!(sum(&[f64::NEG_INFINITY]), f64::NEG_INFINITY);
    assert!(sum(&[f64::INFINITY, f64::NEG_INFINITY]).is_nan());
    assert_eq!(
        sum(&[1.0, f64::EPSILON / 2.0, f64::from_bits(1)]),
        1.000_000_000_000_000_2
    );
    assert_eq!(sum(&[1.0e30, 0.1, -1.0e30]), 0.1);
}

#[test]
fn sum_precise_reproduces_pinned_signed_limb_wraparound() {
    let value = f64::from_bits(2.0_f64.powi(-10).to_bits() - 1);
    let values = vec![value; 129];
    let result = sum(&values);
    assert_eq!(result, -0.124_023_437_500_000_01);
    // The subtraction rounds to exactly -0.25 even though the mathematically
    // equivalent `-256 * value` expression lands one ULP below it.
    assert_eq!(result - 129.0 * value, -0.25);
}

#[test]
fn random_fraction_uses_exactly_the_high_52_bits() {
    for random in [0, 1, u64::MAX, 0x1234_5678_9abc_def0] {
        let value = quickjs_random_fraction(random);
        assert!((0.0..1.0).contains(&value));
        assert_eq!(value * 2.0_f64.powi(52), (value * 2.0_f64.powi(52)).trunc());
    }
    assert_eq!(quickjs_random_fraction(0), 0.0);
    assert_eq!(quickjs_random_fraction(u64::MAX), 1.0 - 2.0_f64.powi(-52));
}

#[test]
fn global_math_is_realm_aware_and_materializes_only_on_get() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_global = first.global_object().unwrap();
    let second_global = second.global_object().unwrap();
    let key = runtime.intern_property_key("Math").unwrap();

    assert!(runtime.has_property(&first_global, &key).unwrap());
    assert!(
        runtime
            .own_property_keys(&first_global)
            .unwrap()
            .contains(&key)
    );
    for (global, realm) in [(&first_global, first.realm), (&second_global, second.realm)] {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(global.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot_index].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(AutoInitProperty::Math {
                realm: defining_realm,
            })) if *defining_realm == realm
        ));
    }

    // The PRNG belongs to the realm bootstrap, not to lazy object creation.
    runtime
        .0
        .state
        .borrow_mut()
        .heap
        .next_math_random_u64(first.realm)
        .expect("Math.random was not seeded before the first Math Get");

    let Value::Object(first_math) = first.get_property(&first_global, &key).unwrap() else {
        panic!("first realm Math did not materialize to an object");
    };
    let Value::Object(first_math_again) = first.get_property(&first_global, &key).unwrap() else {
        panic!("materialized Math did not remain an object");
    };
    assert_eq!(first_math, first_math_again);
    assert_eq!(
        runtime.get_prototype_of(&first_math).unwrap(),
        Some(first.object_prototype().unwrap()),
    );
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(first_global.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::Data(RawValue::Object(id))) if *id == first_math.object_id()
        ));
    }

    assert!(
        runtime
            .is_auto_init_own_property(&second_global, &key)
            .unwrap()
    );
    let Value::Object(second_math) = second.get_property(&second_global, &key).unwrap() else {
        panic!("second realm Math did not materialize to an object");
    };
    assert_ne!(first_math, second_math);
    assert_eq!(
        runtime.get_prototype_of(&second_math).unwrap(),
        Some(second.object_prototype().unwrap()),
    );
}

#[test]
fn deleting_lazy_global_math_releases_its_realm_edge_without_materializing() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("Math").unwrap();
    let count_before = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(context.realm)
        .unwrap();

    assert!(runtime.delete_property(&global, &key).unwrap());
    assert!(!runtime.has_own_property(&global, &key).unwrap());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm)
            .unwrap(),
        count_before - 1,
        "deleting lazy Math did not release its defining-realm edge",
    );
}
