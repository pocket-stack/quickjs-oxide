use super::*;

impl Heap {
    /// Read one live string node payload.
    pub fn string(&self, id: StringId) -> Result<&JsString, HeapError> {
        match self.live_node(RawId::String(id))?.data {
            NodeData::String(ref value) => Ok(value),
            NodeData::Object(_)
            | NodeData::Shape(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_)
            | NodeData::BigInt(_) => Err(HeapError::Invariant(
                "typed string lookup reached another node payload",
            )),
        }
    }

    /// Trusted shared read for a live `StringId` held by an owning edge.
    #[inline]
    pub(crate) fn string_fast(&self, id: StringId) -> &JsString {
        match &self.live_node_fast(RawId::String(id)).data {
            NodeData::String(value) => value,
            _ => unreachable!("trusted string handle reached another node payload"),
        }
    }

    /// Read one live BigInt node payload.
    pub fn bigint(&self, id: BigIntId) -> Result<&JsBigInt, HeapError> {
        match self.live_node(RawId::BigInt(id))?.data {
            NodeData::BigInt(ref value) => Ok(value),
            NodeData::Object(_)
            | NodeData::Shape(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_)
            | NodeData::String(_) => Err(HeapError::Invariant(
                "typed bigint lookup reached another node payload",
            )),
        }
    }

    /// Trusted shared read for a live `BigIntId` held by an owning edge.
    #[inline]
    pub(crate) fn bigint_fast(&self, id: BigIntId) -> &JsBigInt {
        match &self.live_node_fast(RawId::BigInt(id)).data {
            NodeData::BigInt(value) => value,
            _ => unreachable!("trusted bigint handle reached another node payload"),
        }
    }
}
