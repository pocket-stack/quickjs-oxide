//! Observable regression coverage for the shared ordinary property kernel.
use crate::engine::api::runtime::Runtime;
use crate::engine::value::Value;

fn check(source: &str) {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(source).unwrap(), Value::Bool(true));
}

#[test]
fn ordinary_property_context_free_set_rejects_proxy_prototype() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(object) = context.eval("Object.create(new Proxy({}, {}))").unwrap() else {
        panic!("expected object")
    };
    let key = runtime.intern_property_key("x").unwrap();
    assert!(matches!(
        runtime.prepare_set_property(&object, &key, Value::Int(1)),
        Err(crate::engine::api::runtime_error::RuntimeError::Invariant(
            "exotic Set requires a realm"
        ))
    ));
}

#[test]
fn ordinary_property_receiver_proxy_preserves_traps_and_rejection_object() {
    check(
        r#"
      var log=[]; var target={x:0};
      var p=new Proxy(target,{
        getOwnPropertyDescriptor(t,k){log.push('get:'+k);return Reflect.getOwnPropertyDescriptor(t,k)},
        defineProperty(t,k,d){log.push('define:'+k);return Reflect.defineProperty(t,k,d)}
      });
      var accepted=Reflect.set({x:1},'x',42,p);
      var frozen=new Proxy(Object.preventExtensions({}),{});
      var a=[];Object.defineProperty(a,'length',{writable:false});
      var ap=new Proxy(a,{});
      var rejected=!Reflect.set(frozen,'x',1)&&!Reflect.set(ap,'0',1);
      var token={}; var threw=false;
      var throwing=new Proxy({}, {defineProperty(){throw token}});
      try{Reflect.set({x:0},'x',1,throwing)}catch(e){threw=e===token}
      accepted&&target.x===42&&log.join(',')==='get:x,define:x'&&rejected&&threw;
    "#,
    );
}

#[test]
fn ordinary_property_accessor_reentry_and_receiver_rules() {
    check(
        r#"
      var calls=0;var o={};
      Object.defineProperty(o,'x',{configurable:true,get(){
        delete this.x; this.y=9; return 7;
      },set(v){calls++;delete this.x;this.x=v}});
      var read=o.x;
      Object.defineProperty(o,'x',{configurable:true,set(v){calls++;delete this.x;this.x=v}});
      o.x=42;
      var r={set x(v){calls+=100}};
      var rejected=!Reflect.set({x:0},'x',1,r);
      var receiver={};var p={set q(v){this.saved=v}};
      var ok=Reflect.set(p,'q',12,receiver);
      read===7&&o.x===42&&calls===1&&rejected&&ok&&receiver.saved===12;
    "#,
    );
}

#[test]
fn ordinary_property_update_relocates_after_conversion() {
    check(
        r#"
      var o={a:1,x:{valueOf(){delete o.a;delete o.x;o.y=7;return 4}},z:3};
      o.x++;
      var first=o.x===5&&o.y===7&&o.z===3;
      o.x={valueOf(){Object.defineProperty(o,'x',{value:9,writable:false});return 2}};
      var accepted=false;try{(function(){'use strict';o.x+=1})()}catch(e){accepted=e instanceof TypeError}
      first&&accepted&&o.x===9;
    "#,
    );
}

#[test]
fn ordinary_property_metadata_does_not_invoke_accessors_and_missing_stays_distinct() {
    check(
        r#"
      var calls=0;var o={get x(){calls++;return undefined}};
      var a=Object.hasOwn(o,'x')&&o.propertyIsEnumerable('x');
      var d=Object.getOwnPropertyDescriptor(o,'x');
      var b=typeof d.get==='function'&&d.set===undefined&&calls===0;
      var p=new Proxy({}, {get(){calls++;return undefined}});
      var c=Object.create(p);var v=c.unknown;
      a&&b&&v===undefined&&calls===1&&o.x===undefined&&calls===2;
    "#,
    );
}

#[test]
fn ordinary_property_value_define_obeys_all_flags_and_same_value() {
    check(
        r#"
      var values=[undefined,null,true,0,-0,1,1.5,NaN,'x',Symbol('s'),{},1n];
      var good=true;
      for(var writable of [false,true])for(var configurable of [false,true])for(var enumerable of [false,true]){
        for(var old of values)for(var next of values){
          var o={};Object.defineProperty(o,'x',{value:old,writable,configurable,enumerable});
          Object.preventExtensions(o);
          var expected=writable||configurable||Object.is(old,next);
          var accepted=Reflect.defineProperty(o,'x',{value:next});
          var d=Object.getOwnPropertyDescriptor(o,'x');
          good=good&&accepted===expected&&Object.is(d.value,expected?next:old)&&d.writable===writable&&d.configurable===configurable&&d.enumerable===enumerable;
        }
      }
      good;
    "#,
    );
}

#[test]
fn ordinary_property_dictionary_symbols_and_self_references() {
    check(
        r#"
      var o={};var s=Symbol('key');var a=Symbol('a'),b=Symbol('b');
      for(var i=0;i<100;i++)o['p'+i]=i;
      delete o.p1;delete o.p50;
      o.p99=o;o[s]=a;o[s]=a;o[s]=b;
      var keys=Reflect.ownKeys(o);
      o.p2=42;
      o.p99=null;
      o.p2===42&&o[s]===b&&o.p99===null&&keys[keys.length-1]===s&&!Object.hasOwn(o,'p1');
    "#,
    );
}

#[test]
fn ordinary_property_array_length_conversion_precedes_readonly_rejection() {
    check(
        r#"
      var a=[];Object.defineProperty(a,'length',{writable:false});
      var calls=0;var rejected=!Reflect.set(a,'length',{valueOf(){calls++;return 0}});
      var token={};var threw=false;
      try{Reflect.set(a,'length',{valueOf(){throw token}})}catch(e){threw=e===token}
      rejected&&calls>0&&threw&&a.length===0;
    "#,
    );
}

#[test]
fn ordinary_property_proxy_forwarding_preserves_rejection_classification() {
    use crate::engine::object::operations::{InternalSetResult, PropertySetRejection};
    use crate::engine::value::conversion::NativeConversion;
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for (source, name, expected) in [
        (
            "new Proxy(Object.preventExtensions({}),{})",
            "x",
            PropertySetRejection::NotExtensible,
        ),
        (
            "var a=[];Object.defineProperty(a,'length',{writable:false});new Proxy(a,{})",
            "0",
            PropertySetRejection::ArrayLengthReadOnly,
        ),
    ] {
        let Value::Object(proxy) = context.eval(source).unwrap() else {
            panic!("expected Proxy")
        };
        let key = runtime.intern_property_key(name).unwrap();
        let outcome = runtime
            .internal_set(
                context.realm,
                &proxy,
                &key,
                Value::Int(1),
                Value::Object(proxy.clone()),
            )
            .unwrap();
        assert!(
            matches!(outcome, NativeConversion::Value(InternalSetResult::Rejected(reason)) if reason == expected)
        );
    }
}

#[test]
fn ordinary_property_replacing_last_heap_edge_reclaims_old_object() {
    use crate::engine::object::{DescriptorField, OrdinaryPropertyDescriptor};
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    let old = runtime.new_object(None).unwrap();
    let old_id = old.object_id();
    let key = runtime.intern_property_key("x").unwrap();
    runtime
        .define_own_property(
            &object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::Object(old)),
                writable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
        .unwrap();
    assert_eq!(
        runtime.0.state.borrow().heap.object_strong_count(old_id),
        Ok(1)
    );
    context.set_property(&object, &key, Value::Null).unwrap();
    assert!(runtime.0.state.borrow().heap.object(old_id).is_err());
}

#[test]
fn ordinary_property_dense_reads_preserve_holes_receivers_and_transitions() {
    check(
        r#"
        var token={}; var a=[token, 2, 3];
        var denseLength=a.length===3 && a[0]===token && Reflect.get(a,'0',{})===token;
        var symbol=Symbol(); a[symbol]=token; a['01']=5; a[2147483648]=6;
        var large=a[2147483648]===6; delete a[2147483648]; a.length=3;
        var own=a[0]===token && Reflect.get(a,'0',{})===token
            && denseLength && a.length===3 && a['01']===5 && a[symbol]===token && large;
        delete a[1];
        var proto=Object.create(Array.prototype);
        Object.defineProperty(proto,'1',{get(){return this.marker},configurable:true});
        Object.setPrototypeOf(a,proto); a.marker=7;
        var hole=a[1]===7 && Reflect.get(a,'1',{marker:9})===9;
        Object.defineProperty(a,'2',{get(){return this.marker+1},configurable:true});
        var accessor=a[2]===8;
        Object.defineProperty(a,'0',{value:token,writable:false});
        own && hole && accessor && a[0]===token && !Reflect.set(a,'0',4);
        "#,
    );
}

#[test]
fn ordinary_property_typed_access_revalidates_after_conversion() {
    check(
        r#"
        var b=new ArrayBuffer(4,{maxByteLength:8});
        var tracking=new Uint8Array(b); var fixed=new Uint8Array(b,0,4);
        tracking[3]={valueOf(){b.resize(2);return 9}};
        var shrunk=tracking[3]===undefined && fixed[0]===undefined;
        tracking[3]={valueOf(){b.resize(8);return 11}};
        var grown=tracking[3]===11 && fixed[3]===11;
        fixed[0]={valueOf(){b.transfer();return 42}};
        var detached=tracking[0]===undefined && fixed[0]===undefined;
        var shared=new Uint16Array(new SharedArrayBuffer(4));
        shared[1]=513;
        shrunk && grown && detached && shared[1]===513;
        "#,
    );
}

#[test]
fn prepared_read_owns_selected_getter_without_repeating_lookup() {
    use crate::engine::object::ordinary::OrdinaryRead;
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(object) = context
        .eval("var calls=0; var object={get x(){calls++;return this.marker}}; object")
        .unwrap()
    else {
        panic!("expected object")
    };
    let receiver = context.eval("({marker:42})").unwrap();
    let key = runtime.intern_property_key("x").unwrap();
    let read = runtime
        .prepare_ordinary_read(&object, &key, receiver)
        .unwrap();
    assert_eq!(context.eval("calls").unwrap(), Value::Int(0));
    context
        .eval("delete object.x; Object.defineProperty(object,'x',{get(){calls+=100;return 9}})")
        .unwrap();
    let OrdinaryRead::Call { getter, receiver } = read else {
        panic!("expected getter")
    };
    assert_eq!(
        context.call(&getter, receiver, &[]).unwrap(),
        Value::Int(42)
    );
    assert_eq!(context.eval("calls").unwrap(), Value::Int(1));
}
