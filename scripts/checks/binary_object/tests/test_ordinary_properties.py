from pathlib import Path
import re
import tempfile
import unittest

from binary_object.context import ScanContext
from binary_object.rules import ordinary_properties, source_setup

ROOT = Path(__file__).resolve().parents[4]


class OrdinaryPropertyContracts(unittest.TestCase):
    def scan(self, edits=()):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in (*ordinary_properties.FILES, *ordinary_properties.S05_FILES, *ordinary_properties.R5_STORAGE_PROTOCOLS):
                source = (ROOT / relative).read_text()
                for path, before, after in edits:
                    if path == relative:
                        self.assertIn(before, source)
                        source = source.replace(before, after)
                target = root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(source)
            context = ScanContext(root)
            source_setup.check(context)
            ordinary_properties.check(context)
            return context.errors

    def test_current_contracts(self):
        self.assertEqual(self.scan(), [])
        # Both rustfmt's multiline trailing comma and the equivalent compact
        # pattern retain exactly the same owned fields and typed reply route.
        path = "src/engine/vm/proxy_get_driver/request/vm.rs"
        self.assertEqual(self.scan([
            (path, "T::Enumerable {\n                object,\n                key,\n                resume,\n            }", "T::Enumerable { object, key, resume }"),
            (path, "T::Read {\n                object,\n                key,\n                receiver,\n                resume,\n            }", "T::Read { object, key, receiver, resume }"),
            (path, "resume: Resume::ForIn(resume),", "resume: Resume::ForIn(resume)"),
            (path, "resume: Resume::Environment(resume),", "resume: Resume::Environment(resume)"),
        ]), [])

    def test_r5_selected_storage_helpers_reject_observable_work(self):
        for path, names in ordinary_properties.R5_STORAGE_PROTOCOLS.items():
            source = (ROOT / path).read_text()
            for name in names:
                with self.subTest(path=path, helper=name):
                    header = re.search(
                        r"fn\s+" + name + r"\s*\([^{}]*\)\s*->[^{}]*\{", source
                    )
                    self.assertIsNotNone(header)
                    before = header.group(0)
                    # internal_get is intentionally outside the older, narrower
                    # whole ordinary_storage.rs three-method deny-list.
                    self.assertTrue(self.scan([
                        (path, before, before + " runtime.internal_get();")
                    ]))

    def test_bad_boundaries_are_rejected(self):
        storage, ordinary, dispatch, runtime, heap, access, proxy_get, proxy_method, proxy_own, proxy_boolean, descriptor, proxy_call, ordinary_set, proxy_set, proxy_define, array_length, number, typed_element, typed_write, proxy_prototype, builtin_prototype, object_builtin, builtin_property, proxy_keys, builtin_predicate, builtin_definitions, builtin_string = ordinary_properties.FILES
        mutations = [
            (builtin_string, "let value = match result {", "runtime.call_internal(); let value = match result {"),
            (builtin_definitions, "let mut selected = Vec::new();", "runtime.internal_get_own_property(); let mut selected = Vec::new();"),
            (builtin_predicate, "let value = match result {", "runtime.internal_get_own_property(); let value = match result {"),
            (builtin_property, "let object = match (kind, target) {", "runtime.internal_own_property_keys(); let object = match (kind, target) {"),
            (builtin_property, "let key = match runtime.property_key_from_primitive", "runtime.to_primitive(); let key = match runtime.property_key_from_primitive"),
            (proxy_keys, "let mut atoms = HashSet::new();", "runtime.internal_is_extensible(); let mut atoms = HashSet::new();"),
            (proxy_keys, "if let Some(key) = state.remaining.next() {", "runtime.internal_get_own_property(); if let Some(key) = state.remaining.next() {"),
            (builtin_prototype, "let target = arguments", "runtime.internal_get_prototype_of(); let target = arguments"),
            (object_builtin, "if self.is_proxy_object(object)? {", "self.internal_set_prototype_of(); if self.is_proxy_object(object)? {"),
            (proxy_prototype, "let name = match &kind {", "runtime.internal_get_prototype_of(); let name = match &kind {"),
            (proxy_prototype, "if value != prototype {", "runtime.internal_set_prototype_of(); if value != prototype {"),
            (proxy_boolean, "let step = MethodStep::start(runtime, realm, object, name)?;", "runtime.internal_prevent_extensions(); let step = MethodStep::start(runtime, realm, object, name)?;"),
            (proxy_boolean, "let (rooted, key, deleting) = match self.phase {", "runtime.internal_delete_property(); let (rooted, key, deleting) = match self.phase {"),
            (access, 'self.validate_value_domain(base, "delete base")?;', 'self.native_to_property_key(); self.validate_value_domain(base, "delete base")?;'),
            (typed_element, "Ok(match step {", "runtime.native_to_bigint(); Ok(match step {"),
            (typed_write, "let result = match result {", "runtime.typed_array_convert_element(); let result = match result {"),
            (dispatch, "let same_receiver = matches!(receiver,", "self.typed_array_convert_element(); let same_receiver = matches!(receiver,"),
            (array_length, "Ok(match value {", "runtime.native_to_number(); Ok(match value {"),
            (number, "Ok(match step {", "runtime.to_primitive(); Ok(match step {"),
            (ordinary_set, "let _operation = runtime.operation();", "runtime.internal_set(); let _operation = runtime.operation();"),
            (proxy_set, "let key_value = runtime.property_key_value(&key)?;", "runtime.proxy_set(); let key_value = runtime.property_key_value(&key)?;"),
            (proxy_define, "let key_value = runtime.property_key_value(&key)?;", "runtime.internal_define_own_property(); let key_value = runtime.property_key_value(&key)?;"),
            (proxy_call, "let guard = ProxyMethodStackGuard::enter(runtime);", "runtime.call_proxy(); let guard = ProxyMethodStackGuard::enter(runtime);"),
            (dispatch, "PreparedHas::Proxy(current.clone())", "PreparedHas::Complete(false)"),
            (proxy_method, "let key = runtime.intern_property_key(name)?;", "runtime.internal_get(); let key = runtime.intern_property_key(name)?;"),
            (proxy_own, "let key_value = runtime.property_key_value(&key)?;", "runtime.internal_get_own_property(); let key_value = runtime.property_key_value(&key)?;"),
            (proxy_boolean, "let result = runtime.value_to_boolean(&value)?;", "runtime.internal_is_extensible(); let result = runtime.value_to_boolean(&value)?;"),
            (descriptor, "let key = runtime.intern_property_key(name)?;", "runtime.internal_has_property(); let key = runtime.intern_property_key(name)?;"),
            (proxy_get, "runtime.validate_object_and_key(&proxy, &key)?;", "runtime.internal_get(); runtime.validate_object_and_key(&proxy, &key)?;"),
            (storage, "struct OwnSlot", "pub(crate) struct OwnSlot"),
            # Replace every occurrence to simulate removal of the shared class gate.
            (storage, "ObjectKind::Ordinary", "ObjectKind::ModuleNamespace"),
            (storage, "fn locate(", "fn bad() { self.call_internal(); } fn locate("),
            (dispatch, "impl Runtime {", "fn ordinary_set_fast_path_available() {} impl Runtime {"),
            (ordinary_set, "runtime.validate_value_domain(value,", "runtime.skip_domain(value,"),
            (ordinary_set, "rejected_object.as_ref().unwrap_or(receiver)", "receiver"),
            (ordinary, "use crate::engine::object::ordinary_storage::ReadProbe;", "self.call_internal(); use crate::engine::object::ordinary_storage::ReadProbe;"),
            (access, 'self.validate_value_domain(receiver,', 'self.internal_get(); self.validate_value_domain(receiver,'),
            (runtime, "if !failure.published", "if failure.published"),
            (heap, ".retain_edges_transactionally(&new_edges)", ".skip_retain(&new_edges)"),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.assertTrue(self.scan([mutation]))

    def test_s05_owned_domains_and_consumers_are_connected(self):
        mutations = [
            ("src/engine/builtins/array/callback.rs", "enum CallbackStep", "enum MissingCallbackStep"),
            ("src/engine/builtins/array/sort.rs", "struct SortResume", "struct MissingSortResume"),
            ("src/engine/builtins/array.rs", "callback::finish(", "callback::legacy_finish("),
            ("src/engine/vm/array_driver.rs", "LiteralDefinitionStep::start(", "LiteralDefinitionStep::legacy_start("),
            ("src/engine/vm/with_driver.rs", "EnvironmentStep::has_binding(", "EnvironmentStep::legacy_has_binding("),
            ("src/engine/vm/private_access.rs", "proxy_get_driver::start_vm_call(", "proxy_get_driver::legacy_vm_call("),
            ("src/engine/vm/construct_driver.rs", "proxy_get_driver::start_class_parent(", "proxy_get_driver::legacy_class_parent("),
            ("src/engine/vm/frame_operations.rs", "RunExit::Numeric(kind)", "RunExit::LegacyNumeric(kind)"),
            ("src/engine/vm/proxy_get_driver/request/vm.rs", "Self::SnapshotEnumerable", "Self::OwnFlag"),
            ("src/engine/vm/proxy_get_driver/request/vm.rs", "Self::Read {\n                receiver,", "Self::Read {\n                receiver: Value::Undefined,"),
            ("src/engine/vm/proxy_get_driver/request/object.rs", "Self::DefineOrdinary {", "Self::Define {"),
            ("src/engine/vm/proxy_get_driver/request/scalar.rs", "use super::", "fn regress(runtime: &Runtime) { runtime.call_internal(); }\nuse super::"),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.assertTrue(self.scan([mutation]))

    def test_s05_phases_cannot_hide_a_synchronous_callback(self):
        path = "src/engine/object/object_literal/element.rs"
        for callback in ("call_internal", "to_primitive", "native_to_number", "internal_define_own_property", "define_own_property_in_realm"):
            mutation = (path, "let resume = LiteralDefinitionResume::Key", f"runtime.{callback}(); let resume = LiteralDefinitionResume::Key")
            with self.subTest(callback=callback):
                self.assertTrue(self.scan([mutation]))
        for fragment in (
            "use super::*;",
            "fn bad(runtime: &Runtime) { runtime.call_internal(); }",
            # Test modules must not make later production code invisible.
            "#[cfg(test)] mod ignored { fn local() {} }\nfn bad(runtime: &Runtime) { runtime.call_internal(); }",
            # Only the real typed legacy consumer is exempt, not every finish.
            "fn finish(runtime: &Runtime) { runtime.call_internal(); }",
            "fn bad() { let fake: RuntimeVmHost; }",
        ):
            with self.subTest(fragment=fragment):
                self.assertTrue(self.scan([(path, "pub(crate) enum LiteralDefinitionStep", fragment + "\npub(crate) enum LiteralDefinitionStep")]))

    def test_shared_borrowed_property_domain_entries_cannot_bypass_validation(self):
        set_path = 'src/engine/object/ordinary/set.rs'
        read_path = 'src/engine/object/ordinary.rs'
        mutations = [
            (set_path, 'runtime.validate_object_and_key(object, key)?;', 'runtime.skip_object_and_key(object, key)?;'),
            (set_path, 'runtime.validate_value_domain(receiver,', 'runtime.skip_value_domain(receiver,'),
            (set_path, 'initial_set(runtime, realm, &object, &key, &value, &receiver)?', 'unchecked_set(runtime, realm, &object, &key, &value, &receiver)?'),
            (set_path, 'initial_set(runtime, Some(realm), object, key, &value, &receiver)', 'unchecked_set(runtime, Some(realm), object, key, &value, &receiver)'),
            (read_path, 'self.prepare_ordinary_read_borrowed(object, key, &receiver)', 'self.prepare_ordinary_read_borrowed(object, key, &Value::Undefined)'),
            (read_path, 'self.validate_value_domain(receiver,', 'self.skip_value_domain(receiver,'),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.assertTrue(self.scan([mutation]))

    def test_local_for_in_requires_same_object_guard_and_authenticated_helpers(self):
        path = 'src/engine/vm/for_in/operation.rs'
        mutations = [
            (path, 'if !runtime.is_proxy_object(&object)? =>', 'if runtime.is_proxy_object(&object)? =>'),
            (path, 'if !runtime.is_proxy_object(&object)? =>', '=>'),
            (path, 'let reply = runtime.internal_has_own_property(resume.realm, &object, &key)?;', 'let reply = runtime.internal_has_own_property(resume.realm, &other, &key)?;'),
            (path, 'let keys = runtime.own_property_keys(&object)?;', 'runtime.call_internal(); let keys = runtime.own_property_keys(&object)?;'),
            (path, 'step => return Ok(step),', 'step => return Ok(Self::Complete { value: Value::Undefined, done: None }),'),
            (path, 'if runtime.is_proxy_object(&object)? {', 'if false {'),
            (path, 'runtime.internal_has_own_property(realm, &object, &key)?', 'runtime.internal_has_own_property(realm, &other, &key)?'),
            (path, 'if !runtime.is_proxy_object(&pending.object)? {', 'if !runtime.is_proxy_object(&other)? {'),
            (path, 'if !runtime.is_proxy_object(&prototype)? {', 'if !runtime.is_proxy_object(&other)? {'),
            (path, 'let enumerable = match runtime.internal_snapshot_own_property_is_enumerable(', 'runtime.call_internal(); let enumerable = match runtime.internal_snapshot_own_property_is_enumerable('),
            (path, 'if !runtime.is_proxy_object(&prototype)? {', 'if !runtime.is_proxy_object(&prototype)? { runtime.call_internal();'),
            ('src/engine/object/internal_methods.rs', 'self.proxy_snapshot_if_any(object)\n            .map(|value| value.is_some())', 'Ok(false)'),
            ('src/engine/object/internal_methods.rs', 'return Ok(NativeConversion::Value(flags.is_some()));', 'self.call_internal(); return Ok(NativeConversion::Value(flags.is_some()));'),
            ('src/engine/object/properties.rs', 'self.get_own_property_in_operation(object, key)', 'self.internal_get_own_property(realm, object, key)'),
            ('src/engine/modules/namespace.rs', 'let mut atoms = Vec::new();', 'self.call_internal(); let mut atoms = Vec::new();'),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.assertTrue(self.scan([mutation]))

    def test_local_mutation_delete_requires_same_receiver_and_shared_kernel(self):
        path = 'src/engine/builtins/array/mutation.rs'
        guard = 'MutationAction::Delete(key) if !runtime.is_proxy_object(&self.object)? =>'
        mutations = [
            (path, guard, 'MutationAction::Delete(key) =>'),
            (path, guard, 'MutationAction::Delete(key) if runtime.is_proxy_object(&self.object)? =>'),
            (path, guard, 'MutationAction::Delete(key) if !runtime.is_proxy_object(&other)? =>'),
            (path, 'runtime.internal_delete_property(self.realm, &self.object, &key)?', 'runtime.internal_delete_property(self.realm, &other, &key)?'),
            (path, 'runtime.internal_delete_property(self.realm, &self.object, &key)?', 'runtime.internal_delete_property(self.realm, &self.object, &other_key)?'),
            (path, guard + ' {', guard + ' { runtime.call_internal();'),
            (path, 'self.boolean_once(runtime, reply)?', 'self.boolean_once(runtime, NativeConversion::Value(true))?'),
            (path, 'action => return Ok(self.wait(action)),', 'action => return Ok(MutationStep::Complete(Value::Undefined)),'),
            ('src/engine/object/internal_methods.rs', '.delete_property(object, key)', '.delete_property(other, key)'),
            ('src/engine/object/properties.rs', 'let arguments_index = self', 'self.call_internal(); let arguments_index = self'),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.assertTrue(self.scan([mutation]))

    def test_resident_set_selectors_cannot_hide_a_callback(self):
        path = 'src/engine/object/ordinary/set.rs'
        mutations = [
            ('fn select_receiver(&mut self, runtime: &Runtime) -> Result<SelectedSet, RuntimeError> {', 'fn select_receiver(&mut self, runtime: &Runtime) -> Result<SelectedSet, RuntimeError> { runtime.call_internal();'),
            ('let existing = match result {', 'runtime.internal_get(); let existing = match result {'),
            ('let rejected_object = match result {', 'runtime.internal_set(); let rejected_object = match result {'),
            ('fn descriptor(&self, existing: bool) -> OrdinaryPropertyDescriptor {', 'fn descriptor(&self, existing: bool) -> OrdinaryPropertyDescriptor { runtime.call_internal();'),
        ]
        for before, after in mutations:
            with self.subTest(mutation=before):
                self.assertTrue(self.scan([(path, before, after)]))

    def test_numeric_extraction_preserves_connected_owned_route(self):
        parent = 'src/engine/vm/frame_operations.rs'
        child = 'src/engine/vm/frame_operations/numeric.rs'
        mutations = [
            (parent, 'complete_numeric(runtime, execution, id, kind)', 'legacy_numeric(runtime, execution, id, kind)'),
            (parent, 'complete as complete_numeric', 'legacy_complete as complete_numeric'),
            (child, 'NumericStep::start(kind, left, right)', 'NumericStep::legacy_start(kind, left, right)'),
            (child, 'proxy_get_driver::start_numeric(runtime, execution, id, step, depth)', 'proxy_get_driver::legacy_numeric(runtime, execution, id, step, depth)'),
            (child, 'let right = execution.slots.pop(&mut frame.window)?;', 'runtime.to_primitive(); let right = execution.slots.pop(&mut frame.window)?;'),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.assertTrue(self.scan([mutation]))
