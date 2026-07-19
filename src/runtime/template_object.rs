//! Realm-local materialization of QuickJS tagged-template constants.

use super::*;

impl Runtime {
    /// Materialize the structural equivalent of QuickJS
    /// `js_parse_template(..., call = TRUE)`: a frozen cooked Array with a
    /// non-writable/non-enumerable/non-configurable `raw` property referring
    /// to a second frozen Array. The caller retains the returned root until
    /// bytecode allocation has transactionally retained its constant edge.
    pub(super) fn instantiate_template_object(
        &self,
        realm: ContextId,
        cooked: Box<[Option<JsString>]>,
        raw: Box<[JsString]>,
    ) -> Result<ObjectRef, RuntimeError> {
        if cooked.is_empty() || cooked.len() != raw.len() {
            return Err(RuntimeError::Invariant(
                "template object segment lists were empty or misaligned",
            ));
        }

        let template = self.new_array(realm)?;
        let raw_array = self.new_array(realm)?;
        let raw_key = self.intern_property_key("raw")?;
        if !self.define_own_property(
            &template,
            &raw_key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::Object(raw_array.clone())),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(false),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "fresh template object rejected its raw property",
            ));
        }

        for (index, (cooked, raw)) in cooked
            .into_vec()
            .into_iter()
            .zip(raw.into_vec())
            .enumerate()
        {
            let index = u32::try_from(index).map_err(|_| {
                RuntimeError::Engine(Error::new(ErrorKind::Range, "invalid array length"))
            })?;
            if index == u32::MAX {
                return Err(RuntimeError::Engine(Error::new(
                    ErrorKind::Range,
                    "invalid array length",
                )));
            }
            self.define_template_element(&raw_array, index, Value::String(raw))?;
            self.define_template_element(
                &template,
                index,
                cooked.map_or(Value::Undefined, Value::String),
            )?;
        }

        self.seal_template_array(&raw_array)?;
        self.seal_template_array(&template)?;
        Ok(template)
    }

    fn define_template_element(
        &self,
        array: &ObjectRef,
        index: u32,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(&index.to_string())?;
        if !self.define_own_property(
            array,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(false),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "fresh template Array rejected an indexed segment",
            ));
        }
        Ok(())
    }

    fn seal_template_array(&self, array: &ObjectRef) -> Result<(), RuntimeError> {
        let length = self.intern_property_key("length")?;
        if !self.define_own_property(
            array,
            &length,
            &OrdinaryPropertyDescriptor {
                writable: DescriptorField::Present(false),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "fresh template Array rejected its sealed length",
            ));
        }
        self.prevent_extensions(array)
    }
}
