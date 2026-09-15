"""Path-sensitive ForIn/Array mutation guards and their shared helper proofs.

Only an exact guarded non-Proxy match arm may consume a synchronous internal
query. The rest of the domain still undergoes the general callback scan. The
called helpers are authenticated separately, so moving a callback across a file
boundary does not turn the arm into an unchecked exemption.
"""
import re

DEPENDENCIES = {
    'src/engine/object/internal_methods.rs': ('is_proxy_object', 'proxy_snapshot_if_any', 'internal_has_own_property', 'internal_snapshot_own_property_is_enumerable', 'internal_own_property_is_enumerable', 'internal_get_own_property', 'internal_delete_property'),
    'src/engine/object/properties.rs': ('get_own_property', 'get_own_property_in_operation', 'has_own_property', 'own_property_is_enumerable', 'own_property_keys', 'get_prototype_of', 'delete_property'),
    'src/engine/object/ordinary_storage.rs': ('ordinary_property_flags',),
    'src/engine/modules/namespace.rs': ('module_namespace_own_property_keys',),
}
DEPENDENCY_FILES = tuple(DEPENDENCIES)
# Hashes cover reviewed production function bodies, including their signatures,
# not whole files or tests. Non-Proxy own queries cannot select a Proxy callback;
# namespace reads may throw TDZ but only read VarRef descriptor state.
DEPENDENCY_HASHES = {'src/engine/modules/namespace.rs::module_namespace_own_property_keys': '477cc94d96ff1676f5108fb70e4283fe404d948c3ecc80df40075da73a4ba836',
 'src/engine/object/internal_methods.rs::internal_delete_property': '4af724be932aeb7b64bccc452dd6af4dd1e879d3b55d50c6e6283b9badbc5d49',
 'src/engine/object/internal_methods.rs::internal_get_own_property': 'b6322564baae592064739e7071096abbae15fafb6f7f4edc97b93c6feebb060a',
 'src/engine/object/internal_methods.rs::internal_has_own_property': '9542e5a3ebebe2822c46a0b47114b992a0da36c3bbbcd717d0fe3362f36cd20d',
 'src/engine/object/internal_methods.rs::internal_own_property_is_enumerable': 'c6ffc6586cf7b1eba0f37b9fb741b2b708c4c057f0b94adc0364421e40988478',
 'src/engine/object/internal_methods.rs::internal_snapshot_own_property_is_enumerable': '6a2cc874f9a0cd2512c729db176744eaa272ff0f239e8bfb4b6f97a68a803375',
 'src/engine/object/internal_methods.rs::is_proxy_object': 'dd2fd876496e0aecf34bd737c5a3cc05d13251979b0aa4f0928b65d130fa4abc',
 'src/engine/object/internal_methods.rs::proxy_snapshot_if_any': '5d7970bf48ad72ea94e2c16c91fec2d8fc00edbd61556fde44cbaa2be90061d3',
 'src/engine/object/ordinary_storage.rs::ordinary_property_flags': '725bba583e917958493e67f95b062d1a57d41e5f8fb7d90d4e7dc8505dded6e2',
 'src/engine/object/properties.rs::delete_property': 'd7a952e5e1c20561729b1346dd60581ea6332779dd4a76d0be5c212f078ddcd4',
 'src/engine/object/properties.rs::get_own_property': '93a15afe5d406d89a8823242e28440fea97eabcf512772cc34e9d6aa79a8ea1a',
 'src/engine/object/properties.rs::get_own_property_in_operation': '55a130494427f1d45cfdf24453127378f05296de4de2507140320b912367c14d',
 'src/engine/object/properties.rs::get_prototype_of': '460ff734ab71d51c00e138687d691da1da0fda3dd8323f80bdc8e2c5efc520d8',
 'src/engine/object/properties.rs::has_own_property': '38b67d97a3433823e20db8b10e56bb0dfff07d7eb1e4319726750cafff19e14d',
 'src/engine/object/properties.rs::own_property_is_enumerable': '9a4762441b03bf62025bff8b27b1e2e4be26f7c10848d2198e2e2d1f76b8556a',
 'src/engine/object/properties.rs::own_property_keys': 'bbb27c3305e09c99e33752a709277442e70af2dbf45f815ef03389ba89c0fd54'}


def check_local_dependencies(ctx, sources):
    for relative, names in DEPENDENCIES.items():
        source = sources.get(relative, '')
        for name in names:
            body = ctx.unique_braced_item(source, re.compile(r'\bfn\s+' + name + r'\b[^{};]*\{'),
                                          'for-in-local-dependency', relative + '::' + name)[0]
            ctx.require_normalized_code_sha256('for-in-local-dependency',
                'non-Proxy local helper must retain its reviewed no-JavaScript route: ' + relative + '::' + name,
                body, DEPENDENCY_HASHES[relative + '::' + name])


def check_local_arms(ctx, code):
    body, start, end = ctx.unique_braced_item(code, re.compile(r'\bfn\s+advance_without_callback\b[^{};]*\{'),
                                             'for-in-local-guard', 'ForIn local advancement')
    compact = re.sub(r'\s+', '', body)
    arms = (
        ('Keys', 'object,resume', 'letkeys=runtime.own_property_keys(&object)?;resume.keys(runtime,NativeConversion::Value(keys))?'),
        ('Enumerable', 'object,key,resume,', 'letreply=runtime.internal_snapshot_own_property_is_enumerable(resume.realm,&object,&key,)?;resume.boolean(runtime,reply)?'),
        ('Own', 'object,key,resume,', 'letreply=runtime.internal_has_own_property(resume.realm,&object,&key)?;resume.boolean(runtime,reply)?'),
        ('Prototype', 'object,resume', 'letprototype=runtime.get_prototype_of(&object)?;resume.prototype(runtime,NativeConversion::Value(prototype))?'),
    )
    for name, fields, operations in arms:
        exact = 'Self::' + name + '{' + fields + '}if!runtime.is_proxy_object(&object)?=>{' + operations + '}'
        if compact.count(exact) != 1:
            ctx.fail('for-in-local-guard', name + ' must query only the same guarded non-Proxy object and deliver the selected reply once')
        # This removes only a fully authenticated arm; an injected call, changed
        # guard, changed receiver, alias or extra branch remains visible.
        compact = compact.replace(exact, '')
    if compact.count('step=>returnOk(step),') != 1:
        ctx.fail('for-in-local-guard', 'unhandled/Proxy steps must return their exact selected state')
    if start >= 0:
        code = code[:start] + compact + code[end:]
    # Resident loops keep their owner in place. Authenticate the full branch,
    # including the exact guarded object and selected result/continuation route;
    # never exempt their containing function from the callback scan.
    guards = (
        ('advance', r'if\s+runtime\.is_proxy_object\(&object\)\?\s*\{',
         'b479314a25f22afa1da9823791fa2a4580b0e51ba7ff0f64722c60a155ab19e6'),
        ('snapshot_next', r'if\s+!runtime\.is_proxy_object\(&pending\.object\)\?\s*\{',
         '1cf32f87c53f2b5400ecf89c23321aa63475462ff2d439da1baf9ff157e362c8'),
        ('probe_keys', r'if\s+!runtime\.is_proxy_object\(&prototype\)\?\s*\{',
         'f79392464b1aa661a2605c951ddfe89c5509492a5e8287a1d84ba4f0d35474da'),
    )
    for name, pattern, expected in guards:
        function, start, end = ctx.unique_braced_item(code, re.compile(r'\bfn\s+' + name + r'\b[^{};]*\{'),
                                                     'for-in-local-guard', name)
        branch, branch_start, branch_end = ctx.unique_braced_item(function, re.compile(pattern),
                                                                 'for-in-local-guard', name + ' exact object guard')
        ctx.require_normalized_code_sha256('for-in-local-guard',
            name + ' must preserve its same-object guard and selected result', branch, expected)
        if start < 0 or branch_start < 0 or ctx.normalized_code_sha256(branch) != expected:
            continue
        if name == 'advance':
            # Positive Proxy guard returns its exact pending state. Only the
            # immediately adjacent own check is proven to be non-Proxy.
            suffix = re.sub(r'\s+', '', function[branch_end:])
            call = 'letreply=runtime.internal_has_own_property(realm,&object,&key)?;'
            if not suffix.startswith(call):
                ctx.fail('for-in-local-guard', 'candidate Own must immediately query the same guarded object')
                continue
            function = function[:branch_end] + suffix[len(call):]
        else:
            function = function[:branch_start] + ctx.blank(branch) + function[branch_end:]
        code = code[:start] + function + code[end:]
    return code


def check_mutation_local_delete(ctx, code):
    body, start, end = ctx.unique_braced_item(code, re.compile(r'\bfn\s+drive\b[^{};]*\{'),
                                             'array-mutation-local-guard', 'resident mutation driver')
    compact = re.sub(r'\s+', '', body)
    exact = ('MutationAction::Delete(key)if!runtime.is_proxy_object(&self.0.object)?=>{'
             'letreply=runtime.internal_delete_property(self.0.realm,&self.0.object,&key)?;'
             'self.boolean_once(runtime,reply)?}')
    if compact.count(exact) != 1:
        ctx.fail('array-mutation-local-guard', 'Delete must query only the same guarded non-Proxy receiver and deliver its reply once')
    if compact.count('action=>returnOk(self.wait(action)),') != 1:
        ctx.fail('array-mutation-local-guard', 'unhandled/Proxy mutation must retain its exact selected action')
    # An injected callback, wrong object/key, weakened guard, or replay prevents
    # this exact arm from being removed and remains subject to the global scan.
    if start >= 0:
        return code[:start] + compact.replace(exact, '') + code[end:]
    return code
