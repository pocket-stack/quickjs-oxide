use super::*;

#[test]
fn string_index_selectors_use_pinned_generic_magic_cproto() {
    for selector in [StringIndexOfKind::IndexOf, StringIndexOfKind::LastIndexOf] {
        let descriptor = NativeFunctionId::StringPrototypeIndexOf(selector).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::GenericMagic);
        assert!(!descriptor.cproto.default_is_constructor());
    }
}

#[test]
fn string_index_scan_is_utf16_exact_and_inclusive() {
    let source = JsString::try_from_utf16([0x61, 0xd83d, 0xde00, 0xd800, 0x61]).unwrap();
    let crossed = JsString::try_from_utf16([0xde00, 0xd800]).unwrap();
    let empty = JsString::from_static("");
    let too_long = JsString::from_static("abcdef");

    assert_eq!(scan_string_region(&source, &crossed, 0, 3, 1), 2);
    assert_eq!(scan_string_region(&source, &crossed, 3, 0, -1), 2);
    assert_eq!(scan_string_region(&source, &empty, 4, 4, 1), 4);
    assert_eq!(scan_string_region(&source, &too_long, 0, 0, 1), -1);
}

#[test]
fn string_index_methods_preserve_pinned_positions_and_conversion_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for (source, expected) in [
        (r#""aaa".indexOf("a",NaN)"#, 0),
        (r#""aaa".indexOf("a",-Infinity)"#, 0),
        (r#""aaa".indexOf("a",Infinity)"#, -1),
        (r#""aaa".indexOf("",4)"#, 3),
        (r#""aaa".lastIndexOf("a",NaN)"#, 2),
        (r#""aaa".lastIndexOf("a",-Infinity)"#, 0),
        (r#""aaa".lastIndexOf("a",Infinity)"#, 2),
        (r#""aaa".lastIndexOf("",4)"#, 3),
        (r#""abc".indexOf()"#, -1),
        (r#""abc".lastIndexOf()"#, -1),
    ] {
        assert_eq!(
            context.eval(source).unwrap(),
            Value::Int(expected),
            "{source}"
        );
    }

    let order = context
        .eval(
            r#"(function(){
                var log="";
                var receiver=Object();
                receiver[Symbol.toPrimitive]=function(hint){log+="r:"+hint+",";return "ababa"};
                var search=Object();
                var descriptor=Object();
                descriptor.get=function(){log+="match,";throw "wrong"};
                Object.defineProperty(search,Symbol.match,descriptor);
                search[Symbol.toPrimitive]=function(hint){log+="s:"+hint+",";return "ba"};
                var position=Object();
                position[Symbol.toPrimitive]=function(hint){log+="p:"+hint+",";return 2};
                var forward="".indexOf.call(receiver,search,position);
                var reverse="".lastIndexOf.call(receiver,search,position);
                return forward+"|"+reverse+"|"+log;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        order,
        Value::String(JsString::from_static(
            "3|1|r:string,s:string,p:number,r:string,s:string,p:number,",
        )),
    );
}
