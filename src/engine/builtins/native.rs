//! Native builtin selectors, call protocols, and descriptors; execution stays in the runtime.

use crate::engine::api::error::NativeErrorKind;
use crate::engine::heap::{
    AsyncFunctionResumeKind, AsyncGeneratorResumeKind, ContextId, IteratorConsumerKind,
    IteratorHelperKind, IteratorResumeKind, PromiseReactionKind,
};
use std::hash::Hash;

/// Observable mode selected by `Array.prototype.keys`, `values`, or
/// `entries`. QuickJS stores this in `JSArrayIteratorData.kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayIteratorKind {
    Key,
    Value,
    KeyAndValue,
}

/// Observable projection selected by `Map.prototype.keys`, `values`, or
/// `entries`. QuickJS stores this mode in its Map Iterator class payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapIteratorKind {
    Key,
    Value,
    KeyAndValue,
}

/// Observable projection selected by `Set.prototype.values`/`keys` or
/// `entries`. QuickJS stores values in the shared ordered Map record key slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetIteratorKind {
    Value,
    KeyAndValue,
}

/// Observable comparison and traversal mode selected by
/// `Array.prototype.includes`, `indexOf`, or `lastIndexOf`. QuickJS exposes
/// three generic C functions with one algorithmic kernel per mode; retaining
/// that distinction in the native identity avoids dispatch by property name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArraySearchKind {
    Includes,
    IndexOf,
    LastIndexOf,
}

/// Stringification mode selected by QuickJS's shared `js_array_join` kernel.
/// `join` converts an optional separator, whereas `toLocaleString` always uses
/// a comma and invokes each non-nullish element's locale-string method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayJoinKind {
    Join,
    ToLocaleString,
}

/// Head/tail removal mode selected by QuickJS's shared `js_array_pop` kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayPopKind {
    Pop,
    Shift,
}

/// Tail/head insertion mode selected by QuickJS's shared `js_array_push`
/// kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayPushKind {
    Push,
    Unshift,
}

/// Copy/removal mode selected by QuickJS's shared `js_array_slice` kernel.
/// `slice` only materializes the selected range, whereas `splice` returns the
/// same species-created range before mutating the receiver in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArraySliceKind {
    Slice,
    Splice,
}

/// Observable traversal and result mode selected by the four
/// `Array.prototype.find*` methods. QuickJS passes this as the magic value to
/// one shared generic callback kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayFindKind {
    Find,
    FindIndex,
    FindLast,
    FindLastIndex,
}

/// Modes of QuickJS's shared `js_array_every` callback kernel. The typed
/// selector preserves the upstream branch identity without leaking C magic
/// integers into runtime dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayIterationKind {
    Every,
    Some,
    ForEach,
    Map,
    Filter,
}

/// Direction selected by QuickJS's shared `js_array_reduce` accumulator
/// kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayReduceKind {
    Reduce,
    ReduceRight,
}

/// Mode selected by QuickJS's shared `js_array_flatten` kernel. `flatMap`
/// validates and applies a mapper to the outer source before flattening one
/// level, while `flat` converts its requested depth with `JS_ToInt32Sat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayFlattenKind {
    FlatMap,
    Flat,
}

/// Observable key category selected by `%Object%.getOwnPropertyNames` and
/// `%Object%.getOwnPropertySymbols`. QuickJS uses two thin C wrappers around
/// the same `JS_GetOwnPropertyNames2` kernel; retaining the distinction in the
/// native identity keeps Rust dispatch free of string-name tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectOwnPropertyKeysKind {
    Names,
    Symbols,
}

/// Result shape selected by QuickJS's shared `js_object_keys` implementation
/// for `%Object%.keys`, `%Object%.values`, and `%Object%.entries`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKeysKind {
    Keys,
    Values,
    Entries,
}

/// Operation selected by QuickJS's adjacent `%Object%.isExtensible` and
/// `%Object%.preventExtensions` generic-magic builtins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectExtensibilityKind {
    IsExtensible,
    PreventExtensions,
}

/// Operation selected by QuickJS's four generic-magic Object integrity
/// builtins. The mutation and predicate pairs share the same freeze bit in C.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectIntegrityKind {
    Seal,
    Freeze,
    IsSealed,
    IsFrozen,
}

/// Type-safe replacement for QuickJS's getter/setter magic values shared by
/// the Annex-B `__define*__` and `__lookup*__` method families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectAccessorKind {
    Getter,
    Setter,
}

/// Operation selected by pinned QuickJS's complete `js_reflect_funcs` table.
///
/// Keeping the thirteen entries typed preserves both upstream table order and
/// the Generic versus GenericMagic ABI distinction without dispatching on a
/// mutable JavaScript function name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReflectKind {
    Apply,
    Construct,
    DefineProperty,
    DeleteProperty,
    Get,
    GetOwnPropertyDescriptor,
    GetPrototypeOf,
    Has,
    IsExtensible,
    OwnKeys,
    PreventExtensions,
    Set,
    SetPrototypeOf,
}

/// Operation selected by pinned QuickJS's complete `js_json_funcs` table.
///
/// The typed family keeps the public property order stable while parse,
/// stringify and Raw JSON land as separately reviewable algorithmic slices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JsonNativeKind {
    IsRawJson,
    Parse,
    RawJson,
    Stringify,
}

/// Operation selected by QuickJS's shared `js_math_min_max` generic-magic
/// builtin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathMinMaxKind {
    Min,
    Max,
}

/// QuickJS `JS_CFUNC_f_f` Math functions, in pinned table order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathUnaryKind {
    Abs,
    Floor,
    Ceil,
    Round,
    Sqrt,
    Acos,
    Asin,
    Atan,
    Cos,
    Exp,
    Log,
    Sin,
    Tan,
    Trunc,
    Sign,
    Cosh,
    Sinh,
    Tanh,
    Acosh,
    Asinh,
    Atanh,
    Expm1,
    Log1p,
    Log2,
    Log10,
    Cbrt,
    F16Round,
    FRound,
}

/// QuickJS `JS_CFUNC_f_f_f` Math functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathBinaryKind {
    Atan2,
    Pow,
}

/// The eight pinned QuickJS `get_date_string` formatter modes.
///
/// The typed selector is the Rust equivalent of the C callback's magic value:
/// callers can route on the semantic mode while [`Self::quickjs_magic`] keeps
/// the exact upstream table encoding available for differential assertions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateStringMethod {
    String,
    DateString,
    TimeString,
    UtcString,
    IsoString,
    LocaleString,
    LocaleDateString,
    LocaleTimeString,
}

impl DateStringMethod {
    #[cfg(test)]
    pub const ALL: [Self; 8] = [
        Self::String,
        Self::UtcString,
        Self::IsoString,
        Self::DateString,
        Self::TimeString,
        Self::LocaleString,
        Self::LocaleDateString,
        Self::LocaleTimeString,
    ];

    #[must_use]
    #[cfg(test)]
    pub const fn name(self) -> &'static str {
        match self {
            Self::String => "toString",
            Self::DateString => "toDateString",
            Self::TimeString => "toTimeString",
            Self::UtcString => "toUTCString",
            Self::IsoString => "toISOString",
            Self::LocaleString => "toLocaleString",
            Self::LocaleDateString => "toLocaleDateString",
            Self::LocaleTimeString => "toLocaleTimeString",
        }
    }

    #[must_use]
    #[cfg(test)]
    pub const fn uses_local_time(self) -> bool {
        matches!(
            self,
            Self::String
                | Self::DateString
                | Self::TimeString
                | Self::LocaleString
                | Self::LocaleDateString
                | Self::LocaleTimeString
        )
    }

    /// Pinned `JS_CFUNC_MAGIC_DEF` value from `js_date_proto_funcs`.
    #[must_use]
    #[cfg(test)]
    pub const fn quickjs_magic(self) -> u16 {
        match self {
            Self::String => 0x13,
            Self::DateString => 0x11,
            Self::TimeString => 0x12,
            Self::UtcString => 0x03,
            Self::IsoString => 0x23,
            Self::LocaleString => 0x33,
            Self::LocaleDateString => 0x31,
            Self::LocaleTimeString => 0x32,
        }
    }
}

/// Field selected by QuickJS's shared `get_date_field` callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateGetFieldKind {
    Year,
    FullYear,
    UtcFullYear,
    Month,
    UtcMonth,
    Date,
    UtcDate,
    Hours,
    UtcHours,
    Minutes,
    UtcMinutes,
    Seconds,
    UtcSeconds,
    Milliseconds,
    UtcMilliseconds,
    Day,
    UtcDay,
}

impl DateGetFieldKind {
    pub const ALL: [Self; 17] = [
        Self::Year,
        Self::FullYear,
        Self::UtcFullYear,
        Self::Month,
        Self::UtcMonth,
        Self::Date,
        Self::UtcDate,
        Self::Hours,
        Self::UtcHours,
        Self::Minutes,
        Self::UtcMinutes,
        Self::Seconds,
        Self::UtcSeconds,
        Self::Milliseconds,
        Self::UtcMilliseconds,
        Self::Day,
        Self::UtcDay,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Year => "getYear",
            Self::FullYear => "getFullYear",
            Self::UtcFullYear => "getUTCFullYear",
            Self::Month => "getMonth",
            Self::UtcMonth => "getUTCMonth",
            Self::Date => "getDate",
            Self::UtcDate => "getUTCDate",
            Self::Hours => "getHours",
            Self::UtcHours => "getUTCHours",
            Self::Minutes => "getMinutes",
            Self::UtcMinutes => "getUTCMinutes",
            Self::Seconds => "getSeconds",
            Self::UtcSeconds => "getUTCSeconds",
            Self::Milliseconds => "getMilliseconds",
            Self::UtcMilliseconds => "getUTCMilliseconds",
            Self::Day => "getDay",
            Self::UtcDay => "getUTCDay",
        }
    }

    /// Index in QuickJS's nine-element decomposed Date field array.
    #[must_use]
    pub const fn field_index(self) -> u8 {
        match self {
            Self::Year | Self::FullYear | Self::UtcFullYear => 0,
            Self::Month | Self::UtcMonth => 1,
            Self::Date | Self::UtcDate => 2,
            Self::Hours | Self::UtcHours => 3,
            Self::Minutes | Self::UtcMinutes => 4,
            Self::Seconds | Self::UtcSeconds => 5,
            Self::Milliseconds | Self::UtcMilliseconds => 6,
            Self::Day | Self::UtcDay => 7,
        }
    }

    #[must_use]
    pub const fn uses_local_time(self) -> bool {
        matches!(
            self,
            Self::Year
                | Self::FullYear
                | Self::Month
                | Self::Date
                | Self::Hours
                | Self::Minutes
                | Self::Seconds
                | Self::Milliseconds
                | Self::Day
        )
    }

    #[must_use]
    pub const fn is_legacy_year(self) -> bool {
        matches!(self, Self::Year)
    }

    /// Pinned `JS_CFUNC_MAGIC_DEF` value from `js_date_proto_funcs`.
    #[must_use]
    #[cfg(test)]
    pub const fn quickjs_magic(self) -> u16 {
        ((self.is_legacy_year() as u16) << 8)
            | (self.field_index() as u16) << 4
            | self.uses_local_time() as u16
    }
}

/// Field range and UTC/local mode selected by QuickJS's shared
/// `set_date_field` callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateSetFieldKind {
    Milliseconds,
    UtcMilliseconds,
    Seconds,
    UtcSeconds,
    Minutes,
    UtcMinutes,
    Hours,
    UtcHours,
    Date,
    UtcDate,
    Month,
    UtcMonth,
    FullYear,
    UtcFullYear,
}

impl DateSetFieldKind {
    pub const ALL: [Self; 14] = [
        Self::Milliseconds,
        Self::UtcMilliseconds,
        Self::Seconds,
        Self::UtcSeconds,
        Self::Minutes,
        Self::UtcMinutes,
        Self::Hours,
        Self::UtcHours,
        Self::Date,
        Self::UtcDate,
        Self::Month,
        Self::UtcMonth,
        Self::FullYear,
        Self::UtcFullYear,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Milliseconds => "setMilliseconds",
            Self::UtcMilliseconds => "setUTCMilliseconds",
            Self::Seconds => "setSeconds",
            Self::UtcSeconds => "setUTCSeconds",
            Self::Minutes => "setMinutes",
            Self::UtcMinutes => "setUTCMinutes",
            Self::Hours => "setHours",
            Self::UtcHours => "setUTCHours",
            Self::Date => "setDate",
            Self::UtcDate => "setUTCDate",
            Self::Month => "setMonth",
            Self::UtcMonth => "setUTCMonth",
            Self::FullYear => "setFullYear",
            Self::UtcFullYear => "setUTCFullYear",
        }
    }

    /// First Date field replaced by the first supplied argument.
    #[must_use]
    pub const fn first_field(self) -> u8 {
        match self {
            Self::FullYear | Self::UtcFullYear => 0,
            Self::Month | Self::UtcMonth => 1,
            Self::Date | Self::UtcDate => 2,
            Self::Hours | Self::UtcHours => 3,
            Self::Minutes | Self::UtcMinutes => 4,
            Self::Seconds | Self::UtcSeconds => 5,
            Self::Milliseconds | Self::UtcMilliseconds => 6,
        }
    }

    /// Exclusive end of the consecutive Date field range accepted by this
    /// setter. The published function length is `end_field - first_field`.
    #[must_use]
    pub const fn end_field(self) -> u8 {
        match self {
            Self::Date | Self::UtcDate => 3,
            Self::Month | Self::UtcMonth => 3,
            Self::FullYear | Self::UtcFullYear => 3,
            Self::Hours | Self::UtcHours => 7,
            Self::Minutes | Self::UtcMinutes => 7,
            Self::Seconds | Self::UtcSeconds => 7,
            Self::Milliseconds | Self::UtcMilliseconds => 7,
        }
    }

    #[must_use]
    pub const fn uses_local_time(self) -> bool {
        matches!(
            self,
            Self::Milliseconds
                | Self::Seconds
                | Self::Minutes
                | Self::Hours
                | Self::Date
                | Self::Month
                | Self::FullYear
        )
    }

    #[must_use]
    pub const fn length(self) -> u8 {
        self.end_field() - self.first_field()
    }

    /// Pinned `JS_CFUNC_MAGIC_DEF` value from `js_date_proto_funcs`.
    #[must_use]
    #[cfg(test)]
    pub const fn quickjs_magic(self) -> u16 {
        (self.first_field() as u16) << 8
            | (self.end_field() as u16) << 4
            | self.uses_local_time() as u16
    }
}

/// Typed handler family for every callable in pinned QuickJS's Date tables.
///
/// `TimeValue` deliberately backs both `valueOf` and `getTime`, matching their
/// shared C callback. The per-property name and length remain ordinary
/// function-object metadata rather than part of [`NativeFunctionDescriptor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateNativeKind {
    Constructor,
    Now,
    Parse,
    Utc,
    TimeValue,
    String(DateStringMethod),
    ToPrimitive,
    TimezoneOffset,
    GetField(DateGetFieldKind),
    SetTime,
    SetField(DateSetFieldKind),
    SetYear,
    ToJson,
}

impl DateNativeKind {
    /// The unique published function name for this target. `TimeValue` is the
    /// sole shared target: QuickJS publishes distinct `valueOf` and `getTime`
    /// function objects backed by that same callback, so their names remain
    /// per-object intrinsic-table metadata.
    #[must_use]
    #[cfg(test)]
    pub const fn unique_name(self) -> Option<&'static str> {
        match self {
            Self::Constructor => Some("Date"),
            Self::Now => Some("now"),
            Self::Parse => Some("parse"),
            Self::Utc => Some("UTC"),
            Self::TimeValue => None,
            Self::String(kind) => Some(kind.name()),
            Self::ToPrimitive => Some("[Symbol.toPrimitive]"),
            Self::TimezoneOffset => Some("getTimezoneOffset"),
            Self::GetField(kind) => Some(kind.name()),
            Self::SetTime => Some("setTime"),
            Self::SetField(kind) => Some(kind.name()),
            Self::SetYear => Some("setYear"),
            Self::ToJson => Some("toJSON"),
        }
    }

    /// Published `length` for this handler. Both properties using
    /// `TimeValue` have length zero.
    #[must_use]
    pub const fn length(self) -> u8 {
        match self {
            Self::Constructor | Self::Utc => 7,
            Self::Now
            | Self::TimeValue
            | Self::String(_)
            | Self::TimezoneOffset
            | Self::GetField(_) => 0,
            Self::Parse | Self::ToPrimitive | Self::SetTime | Self::SetYear | Self::ToJson => 1,
            Self::SetField(kind) => kind.length(),
        }
    }
}

/// Typed selector for pinned QuickJS's shared RegExp flag getter.  The order
/// follows the public flag surface rather than exposing the engine's bitmask
/// constants to runtime dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegExpFlagKind {
    HasIndices,
    Global,
    IgnoreCase,
    Multiline,
    DotAll,
    Unicode,
    UnicodeSets,
    Sticky,
}

/// Typed handler family for the published RegExp constructor/prototype
/// surface. `Flag` corresponds to QuickJS's getter-magic callback; all other
/// variants preserve their table's concrete C protocol through
/// [`NativeFunctionId::descriptor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegExpNativeKind {
    Constructor,
    Escape,
    Species,
    Exec,
    Compile,
    Test,
    ToString,
    Replace,
    Match,
    MatchAll,
    Search,
    Split,
    Source,
    Flags,
    Flag(RegExpFlagKind),
}

/// Typed handler family for pinned QuickJS's Map constructor and prototype
/// surface. Iterator-producing methods retain their exact projection mode in
/// the native identity instead of dispatching by property name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapNativeKind {
    Constructor,
    Species,
    GroupBy,
    Set,
    Get,
    GetOrInsert,
    GetOrInsertComputed,
    Has,
    Delete,
    Clear,
    Size,
    ForEach,
    Iterator(MapIteratorKind),
}

/// Typed handler family for pinned QuickJS's Set constructor, prototype, and
/// set-composition surface. Iterator identities retain their result projection
/// instead of dispatching by the property that exposed the callable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetNativeKind {
    Constructor,
    Species,
    GroupBy,
    Add,
    Has,
    Delete,
    Clear,
    Size,
    ForEach,
    IsDisjointFrom,
    IsSubsetOf,
    IsSupersetOf,
    Intersection,
    Difference,
    SymmetricDifference,
    Union,
    Iterator(SetIteratorKind),
}

/// Typed handler family for pinned QuickJS's WeakMap surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeakMapNativeKind {
    Constructor,
    Set,
    Get,
    GetOrInsert,
    GetOrInsertComputed,
    Has,
    Delete,
}

/// Typed handler family for pinned QuickJS's WeakSet surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeakSetNativeKind {
    Constructor,
    Add,
    Has,
    Delete,
}

/// Typed handler family for pinned QuickJS's WeakRef surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeakRefNativeKind {
    Constructor,
    Deref,
}

/// Typed handler family for pinned QuickJS's FinalizationRegistry surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FinalizationRegistryNativeKind {
    Constructor,
    Register,
    Unregister,
}

/// Typed handler family for the Promise constructor and its initial surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PromiseNativeKind {
    Constructor,
    Species,
    Then,
    Catch,
    Finally,
    Resolve,
    Reject,
    All,
    AllSettled,
    Any,
    Try,
    Race,
    WithResolvers,
}

/// Typed handler family for pinned QuickJS's ArrayBuffer constructor,
/// prototype, resizable-buffer, transfer, and species surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayBufferNativeKind {
    Constructor,
    IsView,
    Species,
    ByteLength,
    MaxByteLength,
    Resizable,
    Detached,
    Resize,
    Slice,
    Transfer,
    TransferToFixedLength,
}

/// Typed handler family for pinned QuickJS's SharedArrayBuffer constructor,
/// growable-buffer, slice, and species surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SharedArrayBufferNativeKind {
    Constructor,
    Species,
    ByteLength,
    MaxByteLength,
    Growable,
    Grow,
    Slice,
}

/// Operation selector shared by the integer TypedArray Atomics kernel.
///
/// The order mirrors pinned QuickJS's `AtomicsOpEnum`; retaining a typed
/// selector avoids exposing the upstream integer `magic` values to dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AtomicsOperationKind {
    Add,
    And,
    Or,
    Sub,
    Xor,
    Exchange,
    CompareExchange,
    Load,
}

/// Typed handler family for pinned QuickJS's `%Atomics%` namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AtomicsNativeKind {
    Operation(AtomicsOperationKind),
    Store,
    IsLockFree,
    Pause,
    Wait,
    Notify,
}

/// Element format selected by one DataView get/set native.
///
/// The discriminants stay typed instead of relying on QuickJS's integer
/// `magic` values, while preserving the same eleven dispatch variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataViewElementKind {
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    BigInt64,
    BigUint64,
    Float16,
    Float32,
    Float64,
}

impl DataViewElementKind {
    /// Width of this element representation in backing-store bytes.
    #[must_use]
    #[cfg(test)]
    pub const fn byte_length(self) -> u8 {
        match self {
            Self::Int8 | Self::Uint8 => 1,
            Self::Int16 | Self::Uint16 | Self::Float16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::BigInt64 | Self::BigUint64 | Self::Float64 => 8,
        }
    }
}

/// Typed handler family for the DataView constructor, accessors, and element
/// readers/writers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataViewNativeKind {
    Constructor,
    Buffer,
    ByteLength,
    ByteOffset,
    Get(DataViewElementKind),
    Set(DataViewElementKind),
}

/// Concrete element representation selected by a TypedArray constructor.
///
/// The order deliberately matches QuickJS's contiguous class-id range:
/// Uint8Clamped, signed/unsigned integer widths, BigInt widths, then floating
/// point widths. It is also the stable index into realm prototype roots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TypedArrayElementKind {
    Uint8Clamped,
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    BigInt64,
    BigUint64,
    Float16,
    Float32,
    Float64,
}

impl TypedArrayElementKind {
    pub const COUNT: usize = 12;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Uint8Clamped,
        Self::Int8,
        Self::Uint8,
        Self::Int16,
        Self::Uint16,
        Self::Int32,
        Self::Uint32,
        Self::BigInt64,
        Self::BigUint64,
        Self::Float16,
        Self::Float32,
        Self::Float64,
    ];

    /// Width of one element in backing-store bytes.
    #[must_use]
    pub const fn byte_length(self) -> u8 {
        match self {
            Self::Uint8Clamped | Self::Int8 | Self::Uint8 => 1,
            Self::Int16 | Self::Uint16 | Self::Float16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::BigInt64 | Self::BigUint64 | Self::Float64 => 8,
        }
    }

    /// Whether indexed writes require `ToBigInt` rather than `ToNumber`.
    #[must_use]
    pub const fn is_bigint(self) -> bool {
        matches!(self, Self::BigInt64 | Self::BigUint64)
    }

    /// Pinned QuickJS class/global spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Uint8Clamped => "Uint8ClampedArray",
            Self::Int8 => "Int8Array",
            Self::Uint8 => "Uint8Array",
            Self::Int16 => "Int16Array",
            Self::Uint16 => "Uint16Array",
            Self::Int32 => "Int32Array",
            Self::Uint32 => "Uint32Array",
            Self::BigInt64 => "BigInt64Array",
            Self::BigUint64 => "BigUint64Array",
            Self::Float16 => "Float16Array",
            Self::Float32 => "Float32Array",
            Self::Float64 => "Float64Array",
        }
    }
}

/// Selector for the six concrete `Uint8Array` base64/hex codec builtins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Uint8ArrayCodecKind {
    FromBase64,
    FromHex,
    SetFromBase64,
    SetFromHex,
    ToBase64,
    ToHex,
}

/// Typed handler family for the abstract `%TypedArray%` graph, all concrete
/// constructors, and the shared/concrete prototype method tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypedArrayNativeKind {
    BaseConstructor,
    Constructor(TypedArrayElementKind),
    Uint8Codec(Uint8ArrayCodecKind),
    From,
    Of,
    Species,
    Length,
    At,
    With,
    Buffer,
    ByteLength,
    ByteOffset,
    Set,
    Iterator(ArrayIteratorKind),
    ToStringTag,
    CopyWithin,
    Iteration(ArrayIterationKind),
    Reduce(ArrayReduceKind),
    Fill,
    Find(ArrayFindKind),
    Reverse,
    ToReversed,
    Slice,
    Subarray,
    Sort,
    ToSorted,
    Join(ArrayJoinKind),
    Search(ArraySearchKind),
}

/// Selector shared by the paired internal Promise resolving functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PromiseResolvingKind {
    Resolve,
    Reject,
}

/// Selector shared by QuickJS's async-module execution callbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ModuleEvaluationKind {
    Fulfill,
    Reject,
}

/// Selector shared by the two pending dynamic-import Promise handlers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DynamicImportHandlerKind {
    Fulfill,
    Reject,
}

/// Selector for QuickJS's Test262-only `$262.agent` host functions.
#[cfg(feature = "test262-host")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Test262AgentKind {
    Start,
    GetReport,
    Broadcast,
    Report,
    Leaving,
    ReceiveBroadcast,
    Sleep,
    MonotonicNow,
}

/// Runtime-provided callable identities. The enum is stored in heap payloads
/// so native dispatch stays typed and does not rely on function pointers.
// The async-module callback selectors are private implementation details even
// though the surrounding diagnostic identity enum is public.
#[allow(private_interfaces)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeFunctionId {
    FunctionPrototype,
    FunctionConstructor(DynamicFunctionKind),
    GeneratorPrototypeResume(GeneratorResumeKind),
    AsyncGeneratorPrototypeResume(GeneratorResumeKind),
    ArrayConstructor,
    ArrayIsArray,
    ArrayFrom,
    ArrayOf,
    ArraySpeciesGetter,
    ArrayPrototypeIterator(ArrayIteratorKind),
    ArrayPrototypeAt,
    ArrayPrototypeWith,
    ArrayPrototypeConcat,
    ArrayPrototypeIteration(ArrayIterationKind),
    ArrayPrototypeReduce(ArrayReduceKind),
    ArrayPrototypeFill,
    ArrayPrototypeFind(ArrayFindKind),
    ArrayPrototypeSearch(ArraySearchKind),
    ArrayPrototypeJoin(ArrayJoinKind),
    ArrayPrototypeToString,
    ArrayPrototypePop(ArrayPopKind),
    ArrayPrototypePush(ArrayPushKind),
    ArrayPrototypeReverse,
    ArrayPrototypeToReversed,
    ArrayPrototypeSort,
    ArrayPrototypeToSorted,
    ArrayPrototypeSlice(ArraySliceKind),
    ArrayPrototypeToSpliced,
    ArrayPrototypeCopyWithin,
    ArrayPrototypeFlatten(ArrayFlattenKind),
    ArrayIteratorNext,
    ThrowTypeError,
    FunctionPrototypeCall,
    FunctionPrototypeApply,
    FunctionPrototypeBind,
    FunctionPrototypeToString,
    FunctionPrototypeHasInstance,
    FunctionPrototypeFileName,
    FunctionPrototypePosition(FunctionDebugPosition),
    ObjectConstructor,
    ObjectCreate,
    ObjectGetPrototypeOf,
    ObjectSetPrototypeOf,
    ObjectDefineProperty,
    ObjectDefineProperties,
    ObjectGetOwnPropertyKeys(ObjectOwnPropertyKeysKind),
    ObjectGroupBy,
    ObjectKeys(ObjectKeysKind),
    ObjectExtensibility(ObjectExtensibilityKind),
    ObjectGetOwnPropertyDescriptor,
    ObjectGetOwnPropertyDescriptors,
    ObjectIs,
    ObjectAssign,
    ObjectIntegrity(ObjectIntegrityKind),
    ObjectFromEntries,
    ObjectHasOwn,
    ObjectPrototypeToString,
    ObjectPrototypeToLocaleString,
    ObjectPrototypeValueOf,
    ObjectPrototypeHasOwnProperty,
    ObjectPrototypeIsPrototypeOf,
    ObjectPrototypePropertyIsEnumerable,
    ObjectPrototypeProtoGetter,
    ObjectPrototypeProtoSetter,
    ObjectPrototypeDefineAccessor(ObjectAccessorKind),
    ObjectPrototypeLookupAccessor(ObjectAccessorKind),
    ProxyConstructor,
    ProxyRevocable,
    ProxyRevoke,
    Json(JsonNativeKind),
    Reflect(ReflectKind),
    Date(DateNativeKind),
    RegExp(RegExpNativeKind),
    Map(MapNativeKind),
    MapIteratorNext,
    Set(SetNativeKind),
    SetIteratorNext,
    WeakMap(WeakMapNativeKind),
    WeakSet(WeakSetNativeKind),
    WeakRef(WeakRefNativeKind),
    FinalizationRegistry(FinalizationRegistryNativeKind),
    ArrayBuffer(ArrayBufferNativeKind),
    SharedArrayBuffer(SharedArrayBufferNativeKind),
    DataView(DataViewNativeKind),
    TypedArray(TypedArrayNativeKind),
    Atomics(AtomicsNativeKind),
    Promise(PromiseNativeKind),
    AsyncFunctionResume(AsyncFunctionResumeKind),
    AsyncGeneratorResume(AsyncGeneratorResumeKind),
    PromiseResolving(PromiseResolvingKind),
    PromiseCapabilityExecutor,
    PromiseFinallyHandler(PromiseReactionKind),
    PromiseFinallyThunk(PromiseReactionKind),
    PromiseAllResolveElement,
    PromiseAllSettledElement(PromiseReactionKind),
    PromiseAnyRejectElement,
    ModuleEvaluation(ModuleEvaluationKind),
    DynamicImportHandler(DynamicImportHandlerKind),
    AsyncFromSyncIteratorResume(GeneratorResumeKind),
    AsyncFromSyncIteratorUnwrap,
    AsyncFromSyncIteratorClose,
    PrimitiveConstructor(PrimitiveKind),
    StringStatic(StringStaticKind),
    /// QuickJS's test262-only `js_string_codePointRange` helper.
    #[cfg(feature = "test262-host")]
    StringCodePointRange,
    /// QuickJS's test262-only `$262.detachArrayBuffer` host hook.
    #[cfg(feature = "test262-host")]
    Test262DetachArrayBuffer,
    /// QuickJS's test262-only `$262.evalScript` host hook.
    #[cfg(feature = "test262-host")]
    Test262EvalScript,
    /// QuickJS's test262-only `$262.createRealm` host hook.
    #[cfg(feature = "test262-host")]
    Test262CreateRealm,
    /// QuickJS's test262-only callable Annex B `IsHTMLDDA` object.
    #[cfg(feature = "test262-host")]
    Test262IsHtmlDda,
    /// QuickJS's test262-only `$262.gc` host hook.
    #[cfg(feature = "test262-host")]
    Test262Gc,
    /// QuickJS's test262-only `$262.agent` host hooks.
    #[cfg(feature = "test262-host")]
    Test262Agent(Test262AgentKind),
    /// qjs-host `print`, installed explicitly by the CLI rather than as an
    /// ECMAScript intrinsic in every Context.
    QjsPrint,
    /// qjs-host `console.log`; it shares `print` rendering but explicitly
    /// flushes stdout after the line, matching `quickjs-libc.c`.
    QjsConsoleLog,
    PrimitivePrototypeToString(PrimitiveKind),
    PrimitivePrototypeValueOf(PrimitiveKind),
    StringPrototypeCharAt(StringCharAtKind),
    StringPrototypeCharCodeAt,
    StringPrototypeConcat,
    StringPrototypeCodePointAt,
    StringPrototypeWellFormed(StringWellFormedKind),
    StringPrototypeIndexOf(StringIndexOfKind),
    StringPrototypeIncludes(StringIncludesKind),
    StringPrototypeReplace(StringReplaceKind),
    StringPrototypeMatch,
    StringPrototypeMatchAll,
    StringPrototypeSearch,
    StringPrototypeSplit,
    MathMinMax(MathMinMaxKind),
    MathUnary(MathUnaryKind),
    MathBinary(MathBinaryKind),
    MathHypot,
    MathRandom,
    MathImul,
    MathClz32,
    MathSumPrecise,
    StringPrototypeSubrange(StringSubrangeKind),
    StringPrototypeRepeat,
    StringPrototypePad(StringPadKind),
    StringPrototypeTrim(StringTrimKind),
    StringPrototypeCase(StringCaseKind),
    StringPrototypeNormalize,
    StringPrototypeLocaleCompare,
    StringPrototypeCreateHtml(StringCreateHtmlKind),
    IteratorConstructor,
    IteratorConcat,
    IteratorFrom,
    IteratorConstructorAccessor,
    IteratorPrototypeCreateHelper(IteratorHelperKind),
    IteratorPrototypeConsume(IteratorConsumerKind),
    IteratorPrototypeReduce,
    IteratorPrototypeToArray,
    IteratorPrototypeIterator,
    IteratorPrototypeToStringTagGetter,
    IteratorPrototypeToStringTagSetter,
    IteratorHelperResume(IteratorResumeKind),
    IteratorWrapResume(IteratorResumeKind),
    IteratorConcatNext,
    IteratorConcatReturn,
    StringPrototypeIterator,
    StringIteratorNext,
    RegExpStringIteratorNext,
    SymbolRegistry(SymbolRegistryKind),
    SymbolPrototypeDescription,
    BigIntAsN(BigIntAsNKind),
    GlobalEval,
    GlobalNumberParse(NumberParseKind),
    GlobalNumberPredicate(GlobalNumberPredicateKind),
    GlobalUriCodec(GlobalUriCodecKind),
    NumberPredicate(NumberPredicateKind),
    NumberPrototypeFormat(NumberFormatKind),
    ErrorConstructor(ErrorConstructorKind),
    ErrorPrototypeToString,
    ErrorIsError,
    #[cfg(test)]
    ArgumentProbe,
    #[cfg(test)]
    ConstructorProbe,
    #[cfg(test)]
    ConstructorOrFunctionProbe,
    #[cfg(test)]
    ActiveFrameProbe,
}

/// Typed equivalent of QuickJS's magic selector shared by the dynamic
/// Function-family constructors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DynamicFunctionKind {
    Normal,
    Generator,
    Async,
    AsyncGenerator,
}

/// Typed replacement for QuickJS's `GEN_MAGIC_*` selector shared by
/// `%GeneratorPrototype%.next`, `.return`, and `.throw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GeneratorResumeKind {
    Next,
    Return,
    Throw,
}

/// QuickJS primitive wrapper classes which own one realm-local prototype root.
///
/// The complete typed table is present up front, but runtime initialization may
/// leave entries absent until that class reaches a feature-parity milestone.
/// Callers must therefore reject an absent entry instead of falling through to
/// `%Object.prototype%` and silently changing observable behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PrimitiveKind {
    Number,
    String,
    Boolean,
    Symbol,
    BigInt,
}

/// Magic selector shared by `%BigInt%.asUintN` and `%BigInt%.asIntN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BigIntAsNKind {
    AsUintN,
    AsIntN,
}

/// Static operation selected by QuickJS's `%String%` constructor table.
/// Each entry uses the generic C function protocol, while retaining a typed
/// identity for runtime dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringStaticKind {
    FromCharCode,
    FromCodePoint,
    Raw,
}

/// QuickJS's magic selector shared by `String.prototype.at` and
/// `String.prototype.charAt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringCharAtKind {
    At,
    CharAt,
}

/// Typed selector for the adjacent well-formed UTF-16 methods. QuickJS uses
/// separate C functions; retaining one Rust family keeps the shared scan
/// explicit without changing either function's generic C protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringWellFormedKind {
    IsWellFormed,
    ToWellFormed,
}

/// Direction selected by QuickJS's shared `js_string_indexOf` kernel.
/// `indexOf` clamps a saturated Int32 position and scans forward, whereas
/// `lastIndexOf` applies its distinct floating-point default and scans back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringIndexOfKind {
    IndexOf,
    LastIndexOf,
}

/// QuickJS magic selector shared by `String.prototype.includes`, `endsWith`,
/// and `startsWith`. The release's table publishes them in that order with
/// magic values 0, 2, and 1 respectively.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringIncludesKind {
    Includes,
    EndsWith,
    StartsWith,
}

/// QuickJS magic selector shared by `String.prototype.replace` and
/// `String.prototype.replaceAll`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringReplaceKind {
    Replace,
    ReplaceAll,
}

/// Operation selected by the adjacent `substring`, Annex-B `substr`, and
/// `slice` generic String functions. QuickJS publishes three distinct generic
/// C functions; the selector only shares their UTF-16 subrange machinery in
/// Rust and does not change the native function protocol to generic-magic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringSubrangeKind {
    Substring,
    Substr,
    Slice,
}

/// Direction selected by QuickJS's shared `js_string_pad` generic-magic
/// function. The pinned table passes magic one for `padEnd` and zero for
/// `padStart`; typed variants keep that otherwise implicit contract visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringPadKind {
    End,
    Start,
}

/// Ends selected by QuickJS's shared `js_string_trim` generic-magic function.
/// Its bitmask uses one for the leading end and two for the trailing end;
/// typed variants preserve the exact table magic values without exposing raw
/// integers to runtime dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringTrimKind {
    Both,
    End,
    Start,
}

/// Direction selected by QuickJS's shared `js_string_toLowerCase`
/// generic-magic function. The locale-named methods use the same two magic
/// values and intentionally ignore their locale arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringCaseKind {
    Lower,
    Upper,
}

/// Annex-B operation selected by QuickJS's shared `js_string_CreateHTML`
/// generic-magic function. Each variant preserves one table magic value and
/// its corresponding tag/optional attribute pair without exposing raw
/// integers to runtime dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringCreateHtmlKind {
    Anchor,
    Big,
    Blink,
    Bold,
    Fixed,
    FontColor,
    FontSize,
    Italics,
    Link,
    Small,
    Strike,
    Sub,
    Sup,
}

/// Static selector shared by `%Symbol%.for` and `%Symbol%.keyFor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolRegistryKind {
    For,
    KeyFor,
}

impl PrimitiveKind {
    pub const COUNT: usize = 5;

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Global numeric prefix parsers which are also captured by `%Number%` as
/// identity-preserving `parseInt` and `parseFloat` aliases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberParseKind {
    ParseInt,
    ParseFloat,
}

/// Coercing global numeric predicates from QuickJS's base-object table.
/// These stay distinct from the non-coercing static `%Number%` predicates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlobalNumberPredicateKind {
    IsNaN,
    IsFinite,
}

/// QuickJS global URI percent codecs and Annex-B escape helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlobalUriCodecKind {
    DecodeUri,
    DecodeUriComponent,
    EncodeUri,
    EncodeUriComponent,
    Escape,
    Unescape,
}

/// Non-coercing numeric predicates installed as static `%Number%` methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberPredicateKind {
    IsNaN,
    IsFinite,
    IsInteger,
    IsSafeInteger,
}

/// Number-specific prototype formatting operations. The ordinary `toString`
/// and `valueOf` methods continue to use the shared primitive selectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberFormatKind {
    Exponential,
    Fixed,
    Precision,
    LocaleString,
}

/// Typed replacement for the magic selector shared by QuickJS's function
/// definition-position getter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionDebugPosition {
    Line,
    Column,
}

/// Type-safe replacement for QuickJS's integer magic selector on the shared
/// Error constructor handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorConstructorKind {
    Error,
    Native(NativeErrorKind),
}

/// Typed equivalent of QuickJS's C-function protocol selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeCProto {
    Generic,
    GenericMagic,
    Constructor,
    ConstructorMagic,
    ConstructorOrFunction,
    ConstructorOrFunctionMagic,
    UnaryF64,
    BinaryF64,
    Getter,
    Setter,
    GetterMagic,
    #[expect(
        dead_code,
        reason = "Recognized by validation; current producers do not emit this form."
    )]
    SetterMagic,
    IteratorNext,
}

impl NativeCProto {
    /// QuickJS initializes the mutable constructor bit directly from cproto.
    /// Embedders may change the bit later without changing this protocol.
    #[must_use]
    pub const fn default_is_constructor(self) -> bool {
        matches!(
            self,
            Self::Constructor
                | Self::ConstructorMagic
                | Self::ConstructorOrFunction
                | Self::ConstructorOrFunctionMagic
        )
    }
}

/// Static handler-family metadata. Per-object realm, readable arity and the
/// mutable constructor bit deliberately remain outside this descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeFunctionDescriptor {
    pub cproto: NativeCProto,
}

impl NativeFunctionId {
    /// QuickJS invokes class-call handlers and `JS_NewCFunctionData`
    /// callbacks with the `JSContext` supplied by the caller. Ordinary C
    /// functions instead switch to the function object's defining realm.
    ///
    /// The Promise resolving pair uses class-call handlers; its capability
    /// executor, finally callbacks, and aggregate element callbacks use
    /// `JS_NewCFunctionData`. The shared `Iterator.prototype.constructor`
    /// accessor is also a CFunctionData callback: it executes in the calling
    /// realm while retaining its defining realm separately.
    #[must_use]
    pub const fn uses_calling_realm(self) -> bool {
        matches!(
            self,
            Self::AsyncFunctionResume(_)
                | Self::AsyncGeneratorResume(_)
                | Self::PromiseResolving(_)
                | Self::PromiseCapabilityExecutor
                | Self::PromiseFinallyHandler(_)
                | Self::PromiseFinallyThunk(_)
                | Self::PromiseAllResolveElement
                | Self::PromiseAllSettledElement(_)
                | Self::PromiseAnyRejectElement
                | Self::ModuleEvaluation(_)
                | Self::DynamicImportHandler(_)
                | Self::AsyncFromSyncIteratorUnwrap
                | Self::AsyncFromSyncIteratorClose
                | Self::ProxyRevoke
                | Self::IteratorConstructorAccessor
        )
    }

    #[must_use]
    pub const fn descriptor(self) -> NativeFunctionDescriptor {
        match self {
            #[cfg(feature = "test262-host")]
            Self::StringCodePointRange
            | Self::Test262DetachArrayBuffer
            | Self::Test262EvalScript
            | Self::Test262CreateRealm
            | Self::Test262IsHtmlDda
            | Self::Test262Gc
            | Self::Test262Agent(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::Generic,
            },
            Self::FunctionPrototype
            | Self::ThrowTypeError
            | Self::FunctionPrototypeCall
            | Self::FunctionPrototypeApply
            | Self::FunctionPrototypeBind
            | Self::FunctionPrototypeToString
            | Self::FunctionPrototypeHasInstance
            | Self::ObjectCreate
            | Self::ObjectSetPrototypeOf
            | Self::ObjectDefineProperties
            | Self::ObjectGetOwnPropertyKeys(_)
            | Self::ObjectGetOwnPropertyDescriptors
            | Self::ObjectIs
            | Self::ObjectAssign
            | Self::ObjectFromEntries
            | Self::ObjectHasOwn
            | Self::ObjectPrototypeToString
            | Self::ObjectPrototypeToLocaleString
            | Self::ObjectPrototypeValueOf
            | Self::ObjectPrototypeHasOwnProperty
            | Self::ObjectPrototypeIsPrototypeOf
            | Self::ObjectPrototypePropertyIsEnumerable
            | Self::ProxyRevocable
            | Self::ProxyRevoke
            | Self::Json(_)
            | Self::Date(
                DateNativeKind::Now
                | DateNativeKind::Parse
                | DateNativeKind::Utc
                | DateNativeKind::TimeValue
                | DateNativeKind::ToPrimitive
                | DateNativeKind::TimezoneOffset
                | DateNativeKind::SetTime
                | DateNativeKind::SetYear
                | DateNativeKind::ToJson,
            )
            | Self::RegExp(
                RegExpNativeKind::Escape
                | RegExpNativeKind::Exec
                | RegExpNativeKind::Compile
                | RegExpNativeKind::Test
                | RegExpNativeKind::ToString
                | RegExpNativeKind::Replace
                | RegExpNativeKind::Match
                | RegExpNativeKind::MatchAll
                | RegExpNativeKind::Search
                | RegExpNativeKind::Split,
            )
            | Self::Map(
                MapNativeKind::GroupBy
                | MapNativeKind::Set
                | MapNativeKind::Get
                | MapNativeKind::GetOrInsert
                | MapNativeKind::GetOrInsertComputed
                | MapNativeKind::Has
                | MapNativeKind::Delete
                | MapNativeKind::Clear
                | MapNativeKind::ForEach
                | MapNativeKind::Iterator(_),
            )
            | Self::Set(
                SetNativeKind::GroupBy
                | SetNativeKind::Add
                | SetNativeKind::Has
                | SetNativeKind::Delete
                | SetNativeKind::Clear
                | SetNativeKind::ForEach
                | SetNativeKind::IsDisjointFrom
                | SetNativeKind::IsSubsetOf
                | SetNativeKind::IsSupersetOf
                | SetNativeKind::Intersection
                | SetNativeKind::Difference
                | SetNativeKind::SymmetricDifference
                | SetNativeKind::Union
                | SetNativeKind::Iterator(_),
            )
            | Self::WeakMap(
                WeakMapNativeKind::Set
                | WeakMapNativeKind::Get
                | WeakMapNativeKind::GetOrInsert
                | WeakMapNativeKind::GetOrInsertComputed
                | WeakMapNativeKind::Has
                | WeakMapNativeKind::Delete,
            )
            | Self::WeakSet(
                WeakSetNativeKind::Add | WeakSetNativeKind::Has | WeakSetNativeKind::Delete,
            )
            | Self::WeakRef(WeakRefNativeKind::Deref)
            | Self::FinalizationRegistry(
                FinalizationRegistryNativeKind::Register
                | FinalizationRegistryNativeKind::Unregister,
            )
            | Self::ArrayBuffer(ArrayBufferNativeKind::IsView)
            | Self::Atomics(
                AtomicsNativeKind::Store
                | AtomicsNativeKind::IsLockFree
                | AtomicsNativeKind::Pause
                | AtomicsNativeKind::Wait
                | AtomicsNativeKind::Notify,
            )
            | Self::TypedArray(
                TypedArrayNativeKind::From
                | TypedArrayNativeKind::Of
                | TypedArrayNativeKind::Uint8Codec(_)
                | TypedArrayNativeKind::At
                | TypedArrayNativeKind::With
                | TypedArrayNativeKind::Set
                | TypedArrayNativeKind::CopyWithin
                | TypedArrayNativeKind::Fill
                | TypedArrayNativeKind::Reverse
                | TypedArrayNativeKind::ToReversed
                | TypedArrayNativeKind::Slice
                | TypedArrayNativeKind::Subarray
                | TypedArrayNativeKind::Sort
                | TypedArrayNativeKind::ToSorted,
            )
            | Self::Promise(
                PromiseNativeKind::Then
                | PromiseNativeKind::Catch
                | PromiseNativeKind::Finally
                | PromiseNativeKind::Resolve
                | PromiseNativeKind::Reject
                | PromiseNativeKind::All
                | PromiseNativeKind::AllSettled
                | PromiseNativeKind::Any
                | PromiseNativeKind::Try
                | PromiseNativeKind::Race
                | PromiseNativeKind::WithResolvers,
            )
            | Self::AsyncFunctionResume(_)
            | Self::AsyncGeneratorResume(_)
            | Self::PromiseResolving(_)
            | Self::PromiseCapabilityExecutor
            | Self::PromiseFinallyHandler(_)
            | Self::PromiseFinallyThunk(_)
            | Self::PromiseAllResolveElement
            | Self::PromiseAllSettledElement(_)
            | Self::PromiseAnyRejectElement
            | Self::ModuleEvaluation(_)
            | Self::DynamicImportHandler(_)
            | Self::AsyncFromSyncIteratorUnwrap
            | Self::AsyncFromSyncIteratorClose
            | Self::Reflect(
                ReflectKind::Apply
                | ReflectKind::Construct
                | ReflectKind::DeleteProperty
                | ReflectKind::Get
                | ReflectKind::Has
                | ReflectKind::OwnKeys
                | ReflectKind::Set
                | ReflectKind::SetPrototypeOf,
            )
            | Self::PrimitivePrototypeToString(_)
            | Self::PrimitivePrototypeValueOf(_)
            | Self::StringStatic(_)
            | Self::QjsPrint
            | Self::QjsConsoleLog
            | Self::StringPrototypeCharCodeAt
            | Self::StringPrototypeConcat
            | Self::StringPrototypeCodePointAt
            | Self::StringPrototypeWellFormed(_)
            | Self::StringPrototypeSplit
            | Self::MathHypot
            | Self::MathRandom
            | Self::MathImul
            | Self::MathClz32
            | Self::MathSumPrecise
            | Self::StringPrototypeSubrange(_)
            | Self::StringPrototypeRepeat
            | Self::StringPrototypeNormalize
            | Self::StringPrototypeLocaleCompare
            | Self::IteratorConcat
            | Self::IteratorFrom
            | Self::IteratorConstructorAccessor
            | Self::IteratorPrototypeCreateHelper(_)
            | Self::IteratorPrototypeConsume(_)
            | Self::IteratorPrototypeReduce
            | Self::IteratorPrototypeToArray
            | Self::IteratorPrototypeIterator
            | Self::IteratorHelperResume(_)
            | Self::IteratorWrapResume(_)
            | Self::IteratorConcatReturn
            | Self::StringPrototypeIterator
            | Self::ArrayIsArray
            | Self::ArrayFrom
            | Self::ArrayOf
            | Self::ArrayPrototypeIterator(_)
            | Self::ArrayPrototypeAt
            | Self::ArrayPrototypeWith
            | Self::ArrayPrototypeConcat
            | Self::ArrayPrototypeFill
            | Self::ArrayPrototypeSearch(_)
            | Self::ArrayPrototypeToString
            | Self::ArrayPrototypeReverse
            | Self::ArrayPrototypeToReversed
            | Self::ArrayPrototypeSort
            | Self::ArrayPrototypeToSorted
            | Self::ArrayPrototypeToSpliced
            | Self::ArrayPrototypeCopyWithin
            | Self::SymbolRegistry(_)
            | Self::GlobalEval
            | Self::GlobalNumberParse(_)
            | Self::GlobalNumberPredicate(_)
            | Self::NumberPredicate(_)
            | Self::NumberPrototypeFormat(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::Generic,
            },
            Self::ObjectGetPrototypeOf
            | Self::ObjectDefineProperty
            | Self::ObjectGroupBy
            | Self::ObjectKeys(_)
            | Self::ObjectExtensibility(_)
            | Self::ObjectGetOwnPropertyDescriptor
            | Self::ObjectIntegrity(_)
            | Self::ObjectPrototypeDefineAccessor(_)
            | Self::ObjectPrototypeLookupAccessor(_)
            | Self::AsyncGeneratorPrototypeResume(_)
            | Self::AsyncFromSyncIteratorResume(_)
            | Self::Date(
                DateNativeKind::String(_)
                | DateNativeKind::GetField(_)
                | DateNativeKind::SetField(_),
            )
            | Self::Reflect(
                ReflectKind::DefineProperty
                | ReflectKind::GetOwnPropertyDescriptor
                | ReflectKind::GetPrototypeOf
                | ReflectKind::IsExtensible
                | ReflectKind::PreventExtensions,
            )
            | Self::MathMinMax(_)
            | Self::StringPrototypeCharAt(_)
            | Self::StringPrototypeIndexOf(_)
            | Self::StringPrototypeIncludes(_)
            | Self::StringPrototypeReplace(_)
            | Self::StringPrototypeMatch
            | Self::StringPrototypeMatchAll
            | Self::StringPrototypeSearch
            | Self::StringPrototypePad(_)
            | Self::StringPrototypeTrim(_)
            | Self::StringPrototypeCase(_)
            | Self::StringPrototypeCreateHtml(_)
            | Self::ArrayPrototypeFind(_)
            | Self::ArrayPrototypeIteration(_)
            | Self::ArrayPrototypeReduce(_)
            | Self::ArrayPrototypeFlatten(_)
            | Self::ArrayPrototypeJoin(_)
            | Self::ArrayPrototypePop(_)
            | Self::ArrayPrototypePush(_)
            | Self::ArrayPrototypeSlice(_)
            | Self::ArrayBuffer(
                ArrayBufferNativeKind::Resize
                | ArrayBufferNativeKind::Slice
                | ArrayBufferNativeKind::Transfer
                | ArrayBufferNativeKind::TransferToFixedLength,
            )
            | Self::SharedArrayBuffer(
                SharedArrayBufferNativeKind::Grow | SharedArrayBufferNativeKind::Slice,
            )
            | Self::DataView(DataViewNativeKind::Get(_) | DataViewNativeKind::Set(_)) => {
                NativeFunctionDescriptor {
                    cproto: NativeCProto::GenericMagic,
                }
            }
            Self::Atomics(AtomicsNativeKind::Operation(_)) => NativeFunctionDescriptor {
                cproto: NativeCProto::GenericMagic,
            },
            Self::TypedArray(
                TypedArrayNativeKind::Iterator(_)
                | TypedArrayNativeKind::Iteration(_)
                | TypedArrayNativeKind::Reduce(_)
                | TypedArrayNativeKind::Find(_)
                | TypedArrayNativeKind::Join(_)
                | TypedArrayNativeKind::Search(_),
            ) => NativeFunctionDescriptor {
                cproto: NativeCProto::GenericMagic,
            },
            Self::GlobalUriCodec(
                GlobalUriCodecKind::DecodeUri
                | GlobalUriCodecKind::DecodeUriComponent
                | GlobalUriCodecKind::EncodeUri
                | GlobalUriCodecKind::EncodeUriComponent,
            ) => NativeFunctionDescriptor {
                cproto: NativeCProto::GenericMagic,
            },
            Self::BigIntAsN(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::GenericMagic,
            },
            Self::GlobalUriCodec(GlobalUriCodecKind::Escape | GlobalUriCodecKind::Unescape) => {
                NativeFunctionDescriptor {
                    cproto: NativeCProto::Generic,
                }
            }
            Self::FunctionConstructor(_) | Self::PrimitiveConstructor(_) => {
                NativeFunctionDescriptor {
                    cproto: NativeCProto::ConstructorOrFunctionMagic,
                }
            }
            Self::TypedArray(TypedArrayNativeKind::BaseConstructor) => NativeFunctionDescriptor {
                cproto: NativeCProto::ConstructorOrFunction,
            },
            Self::TypedArray(TypedArrayNativeKind::Constructor(_)) => NativeFunctionDescriptor {
                cproto: NativeCProto::ConstructorMagic,
            },
            Self::ArrayConstructor
            | Self::ObjectConstructor
            | Self::IteratorConstructor
            | Self::Date(DateNativeKind::Constructor)
            | Self::RegExp(RegExpNativeKind::Constructor)
            | Self::WeakRef(WeakRefNativeKind::Constructor)
            | Self::FinalizationRegistry(FinalizationRegistryNativeKind::Constructor) => {
                NativeFunctionDescriptor {
                    cproto: NativeCProto::ConstructorOrFunction,
                }
            }
            Self::Map(MapNativeKind::Constructor)
            | Self::Set(SetNativeKind::Constructor)
            | Self::WeakMap(WeakMapNativeKind::Constructor)
            | Self::WeakSet(WeakSetNativeKind::Constructor)
            | Self::ArrayBuffer(ArrayBufferNativeKind::Constructor)
            | Self::SharedArrayBuffer(SharedArrayBufferNativeKind::Constructor)
            | Self::DataView(DataViewNativeKind::Constructor)
            | Self::Promise(PromiseNativeKind::Constructor)
            | Self::ProxyConstructor => NativeFunctionDescriptor {
                cproto: NativeCProto::Constructor,
            },
            Self::FunctionPrototypeFileName
            | Self::ObjectPrototypeProtoGetter
            | Self::RegExp(
                RegExpNativeKind::Species | RegExpNativeKind::Source | RegExpNativeKind::Flags,
            )
            | Self::Map(MapNativeKind::Species | MapNativeKind::Size)
            | Self::Set(SetNativeKind::Species | SetNativeKind::Size)
            | Self::ArrayBuffer(ArrayBufferNativeKind::Species | ArrayBufferNativeKind::Detached)
            | Self::SharedArrayBuffer(SharedArrayBufferNativeKind::Species)
            | Self::DataView(
                DataViewNativeKind::Buffer
                | DataViewNativeKind::ByteLength
                | DataViewNativeKind::ByteOffset,
            )
            | Self::TypedArray(
                TypedArrayNativeKind::Species
                | TypedArrayNativeKind::Length
                | TypedArrayNativeKind::Buffer
                | TypedArrayNativeKind::ByteLength
                | TypedArrayNativeKind::ByteOffset
                | TypedArrayNativeKind::ToStringTag,
            )
            | Self::Promise(PromiseNativeKind::Species) => NativeFunctionDescriptor {
                cproto: NativeCProto::Getter,
            },
            Self::ObjectPrototypeProtoSetter => NativeFunctionDescriptor {
                cproto: NativeCProto::Setter,
            },
            Self::SymbolPrototypeDescription | Self::ArraySpeciesGetter => {
                NativeFunctionDescriptor {
                    cproto: NativeCProto::Getter,
                }
            }
            Self::IteratorPrototypeToStringTagGetter => NativeFunctionDescriptor {
                cproto: NativeCProto::Getter,
            },
            Self::IteratorPrototypeToStringTagSetter => NativeFunctionDescriptor {
                cproto: NativeCProto::Setter,
            },
            Self::StringIteratorNext
            | Self::RegExpStringIteratorNext
            | Self::ArrayIteratorNext
            | Self::MapIteratorNext
            | Self::SetIteratorNext
            | Self::IteratorConcatNext
            | Self::GeneratorPrototypeResume(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::IteratorNext,
            },
            Self::FunctionPrototypePosition(_)
            | Self::RegExp(RegExpNativeKind::Flag(_))
            | Self::ArrayBuffer(
                ArrayBufferNativeKind::ByteLength
                | ArrayBufferNativeKind::MaxByteLength
                | ArrayBufferNativeKind::Resizable,
            )
            | Self::SharedArrayBuffer(
                SharedArrayBufferNativeKind::ByteLength
                | SharedArrayBufferNativeKind::MaxByteLength
                | SharedArrayBufferNativeKind::Growable,
            ) => NativeFunctionDescriptor {
                cproto: NativeCProto::GetterMagic,
            },
            Self::ErrorConstructor(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::ConstructorOrFunctionMagic,
            },
            Self::ErrorPrototypeToString | Self::ErrorIsError => NativeFunctionDescriptor {
                cproto: NativeCProto::Generic,
            },
            Self::MathUnary(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::UnaryF64,
            },
            Self::MathBinary(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::BinaryF64,
            },
            #[cfg(test)]
            Self::ArgumentProbe => NativeFunctionDescriptor {
                cproto: NativeCProto::Generic,
            },
            #[cfg(test)]
            Self::ConstructorProbe => NativeFunctionDescriptor {
                cproto: NativeCProto::Constructor,
            },
            #[cfg(test)]
            Self::ConstructorOrFunctionProbe => NativeFunctionDescriptor {
                cproto: NativeCProto::ConstructorOrFunction,
            },
            #[cfg(test)]
            Self::ActiveFrameProbe => NativeFunctionDescriptor {
                cproto: NativeCProto::Generic,
            },
        }
    }
}

/// Per-object native callable metadata. The own `length` property remains an
/// independent ordinary property and may be modified without affecting
/// `min_readable_args`, just as in QuickJS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeFunctionData {
    pub target: NativeFunctionId,
    /// `None` exists only during `%Function.prototype%` realm bootstrap.
    pub realm: Option<ContextId>,
    pub min_readable_args: u8,
}

#[cfg(all(test, feature = "test262-host"))]
mod tests {
    use super::*;
    #[test]
    fn test262_realm_helpers_are_defining_realm_generic_functions() {
        for target in [
            NativeFunctionId::Test262EvalScript,
            NativeFunctionId::Test262CreateRealm,
            NativeFunctionId::Test262IsHtmlDda,
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
            assert!(!target.descriptor().cproto.default_is_constructor());
            assert!(!target.uses_calling_realm());
        }
    }
}
