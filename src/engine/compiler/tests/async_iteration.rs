use super::*;

#[test]
fn for_await_requires_the_contextual_keyword_and_an_of_head() {
    for source in [
        "for await (var value of values) {}",
        "function ordinary(){ for await (var value of values) {} }",
        "function* generator(){ for await (var value of values) {} }",
        r"async function escaped(){ for aw\u0061it (var value of values) {} }",
    ] {
        let error = compile_unlinked_script(source)
            .expect_err("non-keyword/non-async for-await source unexpectedly compiled");
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}: {error}");
        assert_ne!(
            error.kind(),
            ErrorKind::Unsupported,
            "{source:?} retained the old unsupported frontier"
        );
    }

    for source in [
        "async function ordinary(values){ for await (var value of values) {} }",
        "async function* generator(values){ for await (var value of values) {} }",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("valid async for-await source rejected: {error}"));
    }

    let for_in =
        compile_unlinked_script("async function f(){ for await (var value in values) {} }")
            .expect_err("for-await-in unexpectedly compiled");
    assert_eq!(for_in.kind(), ErrorKind::Syntax);
    assert_eq!(
        for_in.message(),
        "'for await' loop should be used with 'of'"
    );

    for source in [
        "async function f(){ for await (var value = 1 of values) {} }",
        "async function f(){ for await (;;) {} }",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().kind(),
            ErrorKind::Syntax,
            "{source:?}"
        );
    }

    // QuickJS's ordinary for-of ambiguity guard is deliberately disabled for
    // for-await, where `async` is a valid assignment target.
    compile_unlinked_script(
        "async function f(values){ var async; for await (async of values) {} }",
    )
    .expect("for-await should accept an `async` assignment target");
    for source in [
        "function f(values){ var async; for (async of values) {} }",
        "function f(values){ var async; for (async /* comment */ of values) {} }",
        "function f(values){ var async; for (async\n of values) {} }",
        "function f(values){ var async; for (async // comment\n of values) {} }",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            "'for of' expression cannot start with 'async'",
            "{source:?}"
        );
    }
    for source in [
        "function f(values){ var async; for ((async) of values) {} }",
        r"function f(values){ var \u0061sync; for (\u0061sync of values) {} }",
        "function f(values){ var async={}; for (async.value of values) {} }",
        "function f(values,key){ var async={}; for (async[key] of values) {} }",
        "function f(values){ var async={of:0}; for (async.of of values) {} }",
        "function f(){ var async=0; for (async <!-- comment\n of [1]) {} }",
        "function f(){ var async=0; for (async\n --> comment\n of [2]) {} }",
    ] {
        compile_unlinked_script(source).unwrap_or_else(|error| {
            panic!("valid async member target rejected {source:?}: {error}")
        });
    }
    for source in [
        "function f(values){ for (async() of values) {} }",
        "function f(values){ for (async?.value of values) {} }",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().kind(),
            ErrorKind::Syntax,
            "invalid async target unexpectedly compiled: {source:?}"
        );
    }
    assert_eq!(
        compile_unlinked_script(r"for (async of\u0061 of [1]) {}")
            .unwrap_err()
            .message(),
        "'for of' expression cannot start with 'async'"
    );
}

#[test]
fn for_await_uses_the_pinned_three_slot_protocol_lowering() {
    let script = compile_unlinked_script(
        "async function read(source) { \
             for await (const value of source) { if (value) continue; break; } \
             return 1; \
         }",
    )
    .unwrap();
    let function = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .find(|function| function.metadata().function_kind == BytecodeFunctionKind::Async)
        .expect("async child function");
    let code = function.code();

    let start = code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::ForAwaitOfStart))
        .expect("for-await start");
    let next = code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::ForAwaitOfNext))
        .expect("for-await next");
    let await_next = code
        .iter()
        .enumerate()
        .skip(next + 1)
        .find_map(|(index, instruction)| matches!(instruction, Instruction::Await).then_some(index))
        .expect("for-await next await");
    let unwrap = code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::IteratorGetValueDone))
        .expect("for-await result unwrap");
    assert!(start < next && next < await_next && await_next < unwrap);
    assert_eq!(
        code.iter()
            .filter(|instruction| matches!(instruction, Instruction::Await))
            .count(),
        1
    );
    assert_eq!(
        code.iter()
            .filter(|instruction| matches!(instruction, Instruction::IteratorClose))
            .count(),
        1
    );
    assert!(!code.iter().any(|instruction| matches!(
        instruction,
        Instruction::ForOfStart | Instruction::ForOfNext(_)
    )));
}

#[test]
fn async_generator_return_hand_lowers_active_iterator_close() {
    let script = compile_unlinked_script(
        "async function* generate(source) { \
             for await (const value of source) { return value; } \
         } \
         async function ordinary(source) { \
             for await (const value of source) { return value; } \
         }",
    )
    .unwrap();
    let generator = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .find(|function| function.metadata().function_kind == BytecodeFunctionKind::AsyncGenerator)
        .expect("async-generator child function");
    let ordinary = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .find(|function| function.metadata().function_kind == BytecodeFunctionKind::Async)
        .expect("async child function");

    assert!(generator.code().windows(11).any(|window| {
        matches!(
            window,
            [
                Instruction::IteratorDetachPreserve,
                Instruction::GetField2(_),
                Instruction::Dup,
                Instruction::IsUndefinedOrNull,
                Instruction::IfTrue(_),
                Instruction::CallMethod(0),
                Instruction::IteratorCheckObject,
                Instruction::Await,
                Instruction::Goto(_),
                Instruction::Drop,
                Instruction::Drop,
            ]
        )
    }));
    assert_eq!(
        generator
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::IteratorDetachPreserve))
            .count(),
        1
    );
    // next-result await, authored AsyncGenerator return-value await, and
    // hand-lowered iterator-return await.
    assert_eq!(
        generator
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Await))
            .count(),
        3
    );

    // Pinned QuickJS only uses the hand-lowered async close for an
    // AsyncGenerator. An ordinary AsyncFunction retains the existing
    // synchronous IteratorClosePreserve return path.
    assert!(
        ordinary
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::IteratorClosePreserve))
    );
    assert!(!ordinary.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::IteratorDetachPreserve | Instruction::IteratorCheckObject
    )));
    assert_eq!(
        ordinary
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Await))
            .count(),
        1
    );
}
