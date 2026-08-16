//! Release-pinned QuickJS final-bytecode opcode catalog.
//!
//! The table mirrors the uppercase `DEF` entries from `quickjs-opcode.h` in
//! QuickJS 2026-06-04 with `SHORT_OPCODES == 1`. Lowercase `def` entries are
//! temporary compiler opcodes and are intentionally absent: their numeric
//! values overlap the final short opcodes on the wire.

/// Number of opcodes admitted by the pinned final-bytecode wire format.
pub(in crate::runtime) const PINNED_OPCODE_COUNT: usize = 244;

/// Exact operand layouts declared by the pinned `quickjs-opcode.h`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::runtime) enum OpcodeFormat {
    None,
    NoneInt,
    NoneLoc,
    NoneArg,
    NoneVarRef,
    U8,
    I8,
    Loc8,
    Const8,
    Label8,
    U16,
    I16,
    Label16,
    NPop,
    NPopX,
    NPopU16,
    Loc,
    Arg,
    VarRef,
    U32,
    I32,
    Const,
    Label,
    Atom,
    AtomU8,
    AtomU16,
    AtomLabelU8,
    AtomLabelU16,
    LabelU16,
}

impl OpcodeFormat {
    /// Whether this layout starts with a raw `u32` atom operand.
    #[must_use]
    pub(in crate::runtime) const fn has_atom_operand(self) -> bool {
        self.atom_operand_offset().is_some()
    }

    /// Byte offset of the raw atom operand from the instruction start.
    #[must_use]
    pub(in crate::runtime) const fn atom_operand_offset(self) -> Option<u8> {
        match self {
            Self::Atom | Self::AtomU8 | Self::AtomU16 | Self::AtomLabelU8 | Self::AtomLabelU16 => {
                Some(1)
            }
            _ => None,
        }
    }
}

/// One validated opcode byte in the pinned final-bytecode namespace.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime) struct PinnedOpcode(u8);

impl PinnedOpcode {
    /// Validate a raw opcode byte against the 244-entry final catalog.
    ///
    /// Opcode zero is catalogued because it is part of QuickJS's table. A
    /// bytecode scanner must still reject it because the compiler never emits
    /// the `invalid` opcode.
    #[must_use]
    pub(in crate::runtime) const fn from_byte(byte: u8) -> Option<Self> {
        if (byte as usize) < PINNED_OPCODE_COUNT {
            Some(Self(byte))
        } else {
            None
        }
    }

    /// Return the exact final-bytecode opcode byte.
    #[must_use]
    pub(in crate::runtime) const fn raw(self) -> u8 {
        self.0
    }

    /// Return the pinned descriptor for this opcode.
    #[must_use]
    pub(in crate::runtime) const fn info(self) -> &'static PinnedOpcodeInfo {
        &PINNED_OPCODE_INFO[self.0 as usize]
    }

    #[must_use]
    pub(in crate::runtime) const fn name(self) -> &'static str {
        self.info().name()
    }

    #[must_use]
    pub(in crate::runtime) const fn size(self) -> u8 {
        self.info().size()
    }

    #[must_use]
    pub(in crate::runtime) const fn n_pop(self) -> u8 {
        self.info().n_pop()
    }

    #[must_use]
    pub(in crate::runtime) const fn n_push(self) -> u8 {
        self.info().n_push()
    }

    #[must_use]
    pub(in crate::runtime) const fn format(self) -> OpcodeFormat {
        self.info().format()
    }

    #[must_use]
    pub(in crate::runtime) const fn has_atom_operand(self) -> bool {
        self.info().has_atom_operand()
    }

    #[must_use]
    pub(in crate::runtime) const fn atom_operand_offset(self) -> Option<u8> {
        self.info().atom_operand_offset()
    }
}

/// Immutable descriptor for one pinned final-bytecode opcode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct PinnedOpcodeInfo {
    name: &'static str,
    size: u8,
    n_pop: u8,
    n_push: u8,
    format: OpcodeFormat,
}

impl PinnedOpcodeInfo {
    const fn new(
        name: &'static str,
        size: u8,
        n_pop: u8,
        n_push: u8,
        format: OpcodeFormat,
    ) -> Self {
        Self {
            name,
            size,
            n_pop,
            n_push,
            format,
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn name(self) -> &'static str {
        self.name
    }

    #[must_use]
    pub(in crate::runtime) const fn size(self) -> u8 {
        self.size
    }

    #[must_use]
    pub(in crate::runtime) const fn n_pop(self) -> u8 {
        self.n_pop
    }

    #[must_use]
    pub(in crate::runtime) const fn n_push(self) -> u8 {
        self.n_push
    }

    #[must_use]
    pub(in crate::runtime) const fn format(self) -> OpcodeFormat {
        self.format
    }

    #[must_use]
    pub(in crate::runtime) const fn has_atom_operand(self) -> bool {
        self.format.has_atom_operand()
    }

    #[must_use]
    pub(in crate::runtime) const fn atom_operand_offset(self) -> Option<u8> {
        self.format.atom_operand_offset()
    }
}

#[rustfmt::skip]
const PINNED_OPCODE_INFO: [PinnedOpcodeInfo; PINNED_OPCODE_COUNT] = [
    PinnedOpcodeInfo::new("invalid", 1, 0, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("push_i32", 5, 0, 1, OpcodeFormat::I32),
    PinnedOpcodeInfo::new("push_const", 5, 0, 1, OpcodeFormat::Const),
    PinnedOpcodeInfo::new("fclosure", 5, 0, 1, OpcodeFormat::Const),
    PinnedOpcodeInfo::new("push_atom_value", 5, 0, 1, OpcodeFormat::Atom),
    PinnedOpcodeInfo::new("private_symbol", 5, 0, 1, OpcodeFormat::Atom),
    PinnedOpcodeInfo::new("undefined", 1, 0, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("null", 1, 0, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("push_this", 1, 0, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("push_false", 1, 0, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("push_true", 1, 0, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("object", 1, 0, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("special_object", 2, 0, 1, OpcodeFormat::U8),
    PinnedOpcodeInfo::new("rest", 3, 0, 1, OpcodeFormat::U16),
    PinnedOpcodeInfo::new("drop", 1, 1, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("nip", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("nip1", 1, 3, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("dup", 1, 1, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("dup1", 1, 2, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("dup2", 1, 2, 4, OpcodeFormat::None),
    PinnedOpcodeInfo::new("dup3", 1, 3, 6, OpcodeFormat::None),
    PinnedOpcodeInfo::new("insert2", 1, 2, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("insert3", 1, 3, 4, OpcodeFormat::None),
    PinnedOpcodeInfo::new("insert4", 1, 4, 5, OpcodeFormat::None),
    PinnedOpcodeInfo::new("perm3", 1, 3, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("perm4", 1, 4, 4, OpcodeFormat::None),
    PinnedOpcodeInfo::new("perm5", 1, 5, 5, OpcodeFormat::None),
    PinnedOpcodeInfo::new("swap", 1, 2, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("swap2", 1, 4, 4, OpcodeFormat::None),
    PinnedOpcodeInfo::new("rot3l", 1, 3, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("rot3r", 1, 3, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("rot4l", 1, 4, 4, OpcodeFormat::None),
    PinnedOpcodeInfo::new("rot5l", 1, 5, 5, OpcodeFormat::None),
    PinnedOpcodeInfo::new("call_constructor", 3, 2, 1, OpcodeFormat::NPop),
    PinnedOpcodeInfo::new("call", 3, 1, 1, OpcodeFormat::NPop),
    PinnedOpcodeInfo::new("tail_call", 3, 1, 0, OpcodeFormat::NPop),
    PinnedOpcodeInfo::new("call_method", 3, 2, 1, OpcodeFormat::NPop),
    PinnedOpcodeInfo::new("tail_call_method", 3, 2, 0, OpcodeFormat::NPop),
    PinnedOpcodeInfo::new("array_from", 3, 0, 1, OpcodeFormat::NPop),
    PinnedOpcodeInfo::new("apply", 3, 3, 1, OpcodeFormat::U16),
    PinnedOpcodeInfo::new("return", 1, 1, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("return_undef", 1, 0, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("check_ctor_return", 1, 1, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("check_ctor", 1, 0, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("init_ctor", 1, 0, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("check_brand", 1, 2, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("add_brand", 1, 2, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("return_async", 1, 1, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("throw", 1, 1, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("throw_error", 6, 0, 0, OpcodeFormat::AtomU8),
    PinnedOpcodeInfo::new("eval", 5, 1, 1, OpcodeFormat::NPopU16),
    PinnedOpcodeInfo::new("apply_eval", 3, 2, 1, OpcodeFormat::U16),
    PinnedOpcodeInfo::new("regexp", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("get_super", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("import", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("get_var_undef", 3, 0, 1, OpcodeFormat::VarRef),
    PinnedOpcodeInfo::new("get_var", 3, 0, 1, OpcodeFormat::VarRef),
    PinnedOpcodeInfo::new("put_var", 3, 1, 0, OpcodeFormat::VarRef),
    PinnedOpcodeInfo::new("put_var_init", 3, 1, 0, OpcodeFormat::VarRef),
    PinnedOpcodeInfo::new("get_ref_value", 1, 2, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("put_ref_value", 1, 3, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("get_field", 5, 1, 1, OpcodeFormat::Atom),
    PinnedOpcodeInfo::new("get_field2", 5, 1, 2, OpcodeFormat::Atom),
    PinnedOpcodeInfo::new("put_field", 5, 2, 0, OpcodeFormat::Atom),
    PinnedOpcodeInfo::new("get_private_field", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("put_private_field", 1, 3, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("define_private_field", 1, 3, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("get_array_el", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("get_array_el2", 1, 2, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("get_array_el3", 1, 2, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("put_array_el", 1, 3, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("get_super_value", 1, 3, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("put_super_value", 1, 4, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("define_field", 5, 2, 1, OpcodeFormat::Atom),
    PinnedOpcodeInfo::new("set_name", 5, 1, 1, OpcodeFormat::Atom),
    PinnedOpcodeInfo::new("set_name_computed", 1, 2, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("set_proto", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("set_home_object", 1, 2, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("define_array_el", 1, 3, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("append", 1, 3, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("copy_data_properties", 2, 3, 3, OpcodeFormat::U8),
    PinnedOpcodeInfo::new("define_method", 6, 2, 1, OpcodeFormat::AtomU8),
    PinnedOpcodeInfo::new("define_method_computed", 2, 3, 1, OpcodeFormat::U8),
    PinnedOpcodeInfo::new("define_class", 6, 2, 2, OpcodeFormat::AtomU8),
    PinnedOpcodeInfo::new("define_class_computed", 6, 3, 3, OpcodeFormat::AtomU8),
    PinnedOpcodeInfo::new("get_loc", 3, 0, 1, OpcodeFormat::Loc),
    PinnedOpcodeInfo::new("put_loc", 3, 1, 0, OpcodeFormat::Loc),
    PinnedOpcodeInfo::new("set_loc", 3, 1, 1, OpcodeFormat::Loc),
    PinnedOpcodeInfo::new("get_arg", 3, 0, 1, OpcodeFormat::Arg),
    PinnedOpcodeInfo::new("put_arg", 3, 1, 0, OpcodeFormat::Arg),
    PinnedOpcodeInfo::new("set_arg", 3, 1, 1, OpcodeFormat::Arg),
    PinnedOpcodeInfo::new("get_var_ref", 3, 0, 1, OpcodeFormat::VarRef),
    PinnedOpcodeInfo::new("put_var_ref", 3, 1, 0, OpcodeFormat::VarRef),
    PinnedOpcodeInfo::new("set_var_ref", 3, 1, 1, OpcodeFormat::VarRef),
    PinnedOpcodeInfo::new("set_loc_uninitialized", 3, 0, 0, OpcodeFormat::Loc),
    PinnedOpcodeInfo::new("get_loc_check", 3, 0, 1, OpcodeFormat::Loc),
    PinnedOpcodeInfo::new("put_loc_check", 3, 1, 0, OpcodeFormat::Loc),
    PinnedOpcodeInfo::new("set_loc_check", 3, 1, 1, OpcodeFormat::Loc),
    PinnedOpcodeInfo::new("put_loc_check_init", 3, 1, 0, OpcodeFormat::Loc),
    PinnedOpcodeInfo::new("get_loc_checkthis", 3, 0, 1, OpcodeFormat::Loc),
    PinnedOpcodeInfo::new("get_var_ref_check", 3, 0, 1, OpcodeFormat::VarRef),
    PinnedOpcodeInfo::new("put_var_ref_check", 3, 1, 0, OpcodeFormat::VarRef),
    PinnedOpcodeInfo::new("put_var_ref_check_init", 3, 1, 0, OpcodeFormat::VarRef),
    PinnedOpcodeInfo::new("close_loc", 3, 0, 0, OpcodeFormat::Loc),
    PinnedOpcodeInfo::new("if_false", 5, 1, 0, OpcodeFormat::Label),
    PinnedOpcodeInfo::new("if_true", 5, 1, 0, OpcodeFormat::Label),
    PinnedOpcodeInfo::new("goto", 5, 0, 0, OpcodeFormat::Label),
    PinnedOpcodeInfo::new("catch", 5, 0, 1, OpcodeFormat::Label),
    PinnedOpcodeInfo::new("gosub", 5, 0, 0, OpcodeFormat::Label),
    PinnedOpcodeInfo::new("ret", 1, 1, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("nip_catch", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("to_object", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("to_propkey", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("with_get_var", 10, 1, 0, OpcodeFormat::AtomLabelU8),
    PinnedOpcodeInfo::new("with_put_var", 10, 2, 1, OpcodeFormat::AtomLabelU8),
    PinnedOpcodeInfo::new("with_delete_var", 10, 1, 0, OpcodeFormat::AtomLabelU8),
    PinnedOpcodeInfo::new("with_make_ref", 10, 1, 0, OpcodeFormat::AtomLabelU8),
    PinnedOpcodeInfo::new("with_get_ref", 10, 1, 0, OpcodeFormat::AtomLabelU8),
    PinnedOpcodeInfo::new("make_loc_ref", 7, 0, 2, OpcodeFormat::AtomU16),
    PinnedOpcodeInfo::new("make_arg_ref", 7, 0, 2, OpcodeFormat::AtomU16),
    PinnedOpcodeInfo::new("make_var_ref_ref", 7, 0, 2, OpcodeFormat::AtomU16),
    PinnedOpcodeInfo::new("make_var_ref", 5, 0, 2, OpcodeFormat::Atom),
    PinnedOpcodeInfo::new("for_in_start", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("for_of_start", 1, 1, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("for_await_of_start", 1, 1, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("for_in_next", 1, 1, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("for_of_next", 2, 3, 5, OpcodeFormat::U8),
    PinnedOpcodeInfo::new("for_await_of_next", 1, 3, 4, OpcodeFormat::None),
    PinnedOpcodeInfo::new("iterator_check_object", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("iterator_get_value_done", 1, 2, 3, OpcodeFormat::None),
    PinnedOpcodeInfo::new("iterator_close", 1, 3, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("iterator_next", 1, 4, 4, OpcodeFormat::None),
    PinnedOpcodeInfo::new("iterator_call", 2, 4, 5, OpcodeFormat::U8),
    PinnedOpcodeInfo::new("initial_yield", 1, 0, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("yield", 1, 1, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("yield_star", 1, 1, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("async_yield_star", 1, 1, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("await", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("neg", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("plus", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("dec", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("inc", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("post_dec", 1, 1, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("post_inc", 1, 1, 2, OpcodeFormat::None),
    PinnedOpcodeInfo::new("dec_loc", 2, 0, 0, OpcodeFormat::Loc8),
    PinnedOpcodeInfo::new("inc_loc", 2, 0, 0, OpcodeFormat::Loc8),
    PinnedOpcodeInfo::new("add_loc", 2, 1, 0, OpcodeFormat::Loc8),
    PinnedOpcodeInfo::new("not", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("lnot", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("typeof", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("delete", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("delete_var", 5, 0, 1, OpcodeFormat::Atom),
    PinnedOpcodeInfo::new("mul", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("div", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("mod", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("add", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("sub", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("pow", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("shl", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("sar", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("shr", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("lt", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("lte", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("gt", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("gte", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("instanceof", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("in", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("eq", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("neq", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("strict_eq", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("strict_neq", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("and", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("xor", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("or", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("is_undefined_or_null", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("private_in", 1, 2, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("push_bigint_i32", 5, 0, 1, OpcodeFormat::I32),
    PinnedOpcodeInfo::new("nop", 1, 0, 0, OpcodeFormat::None),
    PinnedOpcodeInfo::new("push_minus1", 1, 0, 1, OpcodeFormat::NoneInt),
    PinnedOpcodeInfo::new("push_0", 1, 0, 1, OpcodeFormat::NoneInt),
    PinnedOpcodeInfo::new("push_1", 1, 0, 1, OpcodeFormat::NoneInt),
    PinnedOpcodeInfo::new("push_2", 1, 0, 1, OpcodeFormat::NoneInt),
    PinnedOpcodeInfo::new("push_3", 1, 0, 1, OpcodeFormat::NoneInt),
    PinnedOpcodeInfo::new("push_4", 1, 0, 1, OpcodeFormat::NoneInt),
    PinnedOpcodeInfo::new("push_5", 1, 0, 1, OpcodeFormat::NoneInt),
    PinnedOpcodeInfo::new("push_6", 1, 0, 1, OpcodeFormat::NoneInt),
    PinnedOpcodeInfo::new("push_7", 1, 0, 1, OpcodeFormat::NoneInt),
    PinnedOpcodeInfo::new("push_i8", 2, 0, 1, OpcodeFormat::I8),
    PinnedOpcodeInfo::new("push_i16", 3, 0, 1, OpcodeFormat::I16),
    PinnedOpcodeInfo::new("push_const8", 2, 0, 1, OpcodeFormat::Const8),
    PinnedOpcodeInfo::new("fclosure8", 2, 0, 1, OpcodeFormat::Const8),
    PinnedOpcodeInfo::new("push_empty_string", 1, 0, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("get_loc8", 2, 0, 1, OpcodeFormat::Loc8),
    PinnedOpcodeInfo::new("put_loc8", 2, 1, 0, OpcodeFormat::Loc8),
    PinnedOpcodeInfo::new("set_loc8", 2, 1, 1, OpcodeFormat::Loc8),
    PinnedOpcodeInfo::new("get_loc0", 1, 0, 1, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("get_loc1", 1, 0, 1, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("get_loc2", 1, 0, 1, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("get_loc3", 1, 0, 1, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("put_loc0", 1, 1, 0, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("put_loc1", 1, 1, 0, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("put_loc2", 1, 1, 0, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("put_loc3", 1, 1, 0, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("set_loc0", 1, 1, 1, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("set_loc1", 1, 1, 1, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("set_loc2", 1, 1, 1, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("set_loc3", 1, 1, 1, OpcodeFormat::NoneLoc),
    PinnedOpcodeInfo::new("get_arg0", 1, 0, 1, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("get_arg1", 1, 0, 1, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("get_arg2", 1, 0, 1, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("get_arg3", 1, 0, 1, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("put_arg0", 1, 1, 0, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("put_arg1", 1, 1, 0, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("put_arg2", 1, 1, 0, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("put_arg3", 1, 1, 0, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("set_arg0", 1, 1, 1, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("set_arg1", 1, 1, 1, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("set_arg2", 1, 1, 1, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("set_arg3", 1, 1, 1, OpcodeFormat::NoneArg),
    PinnedOpcodeInfo::new("get_var_ref0", 1, 0, 1, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("get_var_ref1", 1, 0, 1, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("get_var_ref2", 1, 0, 1, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("get_var_ref3", 1, 0, 1, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("put_var_ref0", 1, 1, 0, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("put_var_ref1", 1, 1, 0, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("put_var_ref2", 1, 1, 0, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("put_var_ref3", 1, 1, 0, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("set_var_ref0", 1, 1, 1, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("set_var_ref1", 1, 1, 1, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("set_var_ref2", 1, 1, 1, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("set_var_ref3", 1, 1, 1, OpcodeFormat::NoneVarRef),
    PinnedOpcodeInfo::new("get_length", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("if_false8", 2, 1, 0, OpcodeFormat::Label8),
    PinnedOpcodeInfo::new("if_true8", 2, 1, 0, OpcodeFormat::Label8),
    PinnedOpcodeInfo::new("goto8", 2, 0, 0, OpcodeFormat::Label8),
    PinnedOpcodeInfo::new("goto16", 3, 0, 0, OpcodeFormat::Label16),
    PinnedOpcodeInfo::new("call0", 1, 1, 1, OpcodeFormat::NPopX),
    PinnedOpcodeInfo::new("call1", 1, 1, 1, OpcodeFormat::NPopX),
    PinnedOpcodeInfo::new("call2", 1, 1, 1, OpcodeFormat::NPopX),
    PinnedOpcodeInfo::new("call3", 1, 1, 1, OpcodeFormat::NPopX),
    PinnedOpcodeInfo::new("is_undefined", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("is_null", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("typeof_is_undefined", 1, 1, 1, OpcodeFormat::None),
    PinnedOpcodeInfo::new("typeof_is_function", 1, 1, 1, OpcodeFormat::None),
];

#[cfg(test)]
mod tests {
    use super::{OpcodeFormat, PINNED_OPCODE_COUNT, PinnedOpcode};

    const ALL_FORMATS: [OpcodeFormat; 29] = [
        OpcodeFormat::None,
        OpcodeFormat::NoneInt,
        OpcodeFormat::NoneLoc,
        OpcodeFormat::NoneArg,
        OpcodeFormat::NoneVarRef,
        OpcodeFormat::U8,
        OpcodeFormat::I8,
        OpcodeFormat::Loc8,
        OpcodeFormat::Const8,
        OpcodeFormat::Label8,
        OpcodeFormat::U16,
        OpcodeFormat::I16,
        OpcodeFormat::Label16,
        OpcodeFormat::NPop,
        OpcodeFormat::NPopX,
        OpcodeFormat::NPopU16,
        OpcodeFormat::Loc,
        OpcodeFormat::Arg,
        OpcodeFormat::VarRef,
        OpcodeFormat::U32,
        OpcodeFormat::I32,
        OpcodeFormat::Const,
        OpcodeFormat::Label,
        OpcodeFormat::Atom,
        OpcodeFormat::AtomU8,
        OpcodeFormat::AtomU16,
        OpcodeFormat::AtomLabelU8,
        OpcodeFormat::AtomLabelU16,
        OpcodeFormat::LabelU16,
    ];

    fn opcode(raw: u8) -> PinnedOpcode {
        PinnedOpcode::from_byte(raw).unwrap()
    }

    #[test]
    fn final_catalog_boundaries_skip_temporary_descriptors() {
        assert_eq!(PINNED_OPCODE_COUNT, 244);
        assert_eq!(opcode(0).name(), "invalid");
        assert_eq!(opcode(0).raw(), 0);
        assert_eq!(opcode(177).name(), "nop");
        assert_eq!(opcode(178).name(), "push_minus1");
        assert_eq!(opcode(243).name(), "typeof_is_function");
        assert_eq!(PinnedOpcode::from_byte(244), None);
        assert_eq!(PinnedOpcode::from_byte(u8::MAX), None);
    }

    #[test]
    fn final_catalog_locks_sizes_stack_effects_and_formats() {
        assert_eq!(opcode(0).size(), 1);
        assert_eq!(opcode(20).n_pop(), 3);
        assert_eq!(opcode(20).n_push(), 6);
        assert_eq!(opcode(49).size(), 6);
        assert_eq!(opcode(49).format(), OpcodeFormat::AtomU8);
        assert_eq!(opcode(113).size(), 10);
        assert_eq!(opcode(113).format(), OpcodeFormat::AtomLabelU8);
        assert_eq!(opcode(118).size(), 7);
        assert_eq!(opcode(118).format(), OpcodeFormat::AtomU16);
        assert_eq!(opcode(151).size(), 5);
        assert_eq!(opcode(151).format(), OpcodeFormat::Atom);
        assert_eq!(opcode(177).format(), OpcodeFormat::None);
        assert_eq!(opcode(178).format(), OpcodeFormat::NoneInt);
        assert_eq!(opcode(243).format(), OpcodeFormat::None);
    }

    #[test]
    fn final_catalog_has_exact_atom_operand_set_at_offset_one() {
        let atom_ids = (0..PINNED_OPCODE_COUNT)
            .map(|raw| opcode(raw as u8))
            .filter(|opcode| opcode.has_atom_operand())
            .map(PinnedOpcode::raw)
            .collect::<Vec<_>>();

        assert_eq!(
            atom_ids,
            [
                4, 5, 49, 61, 62, 63, 73, 74, 81, 83, 84, 113, 114, 115, 116, 117, 118, 119, 120,
                121, 151,
            ]
        );
        for raw in atom_ids {
            assert_eq!(opcode(raw).atom_operand_offset(), Some(1));
        }
        assert_eq!(opcode(3).atom_operand_offset(), None);
    }

    #[test]
    fn all_upstream_formats_are_represented_even_when_not_final() {
        assert_eq!(ALL_FORMATS.len(), 29);
        assert!(!OpcodeFormat::U32.has_atom_operand());
        assert_eq!(OpcodeFormat::AtomLabelU16.atom_operand_offset(), Some(1));
        assert!(!OpcodeFormat::LabelU16.has_atom_operand());
    }
}
