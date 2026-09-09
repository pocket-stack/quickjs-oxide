use super::*;

#[test]
fn for_in_of_array_bindings_use_nested_iterator_records() {
    let for_of = compile_unlinked_script("for(const [a,,b,] of [[1,2,3]])a+b").unwrap();
    assert_eq!(
        for_of
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ForOfStart))
            .count(),
        2
    );
    assert_eq!(
        for_of
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ForOfNext(0)))
            .count(),
        4
    );
    assert_eq!(
        for_of
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::IteratorClose))
            .count(),
        2
    );
    assert_eq!(
        for_of
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::InitializeLocal(_)))
            .count(),
        2
    );

    let for_in = compile_unlinked_script("for(var [a,b] in {ab:1})a+b").unwrap();
    assert_eq!(
        for_in
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ForInStart))
            .count(),
        1
    );
    assert_eq!(
        for_in
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ForOfStart))
            .count(),
        1
    );
    assert_eq!(
        for_in
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ForOfNext(1)))
            .count(),
        2
    );

    for source in [
        "for(var [a]=[1] in {a:1})a",
        "for(let [a]=[1] in {a:1})a",
        "for(const [a]=[1] of [[1]])a",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            format!(
                "a declaration in the head of a for-{} loop can't have an initializer",
                if source.contains(" of ") { "of" } else { "in" }
            ),
            "{source}"
        );
    }
}

#[test]
fn object_binding_declarations_cover_direct_and_loop_surfaces() {
    for (source, expected) in [
        (
            "(function(){var {fixed}={fixed:'v'};let {['computed']:computed}={computed:'l'};const {nested:{value}}={nested:{value:'c'}};return fixed+computed+value})()",
            "vlc",
        ),
        (
            "(function(){var result='';for(var {fixed}={fixed:'v'};;){result+=fixed;break}for(let {['computed']:computed}={computed:'l'};;){result+=computed;break}for(const {nested:{value}}={nested:{value:'c'}};;){result+=value;break}return result})()",
            "vlc",
        ),
        (
            "(function(){var result='';for(var {0:first} in {ab:1})result+=first;for(let {[1]:second} in {ab:1})result+=second;for(const {constructor:{name}} in {ab:1})result+=name;return result})()",
            "abString",
        ),
        (
            "(function(){var result='';for(var {fixed} of [{fixed:'v'}])result+=fixed;for(let {['computed']:computed} of [{computed:'l'}])result+=computed;for(const {nested:{value}} of [{nested:{value:'c'}}])result+=value;return result})()",
            "vlc",
        ),
    ] {
        assert_eq!(
            evaluate_in_context(source),
            Value::String(JsString::from_static(expected)),
            "{source}"
        );
    }
}
