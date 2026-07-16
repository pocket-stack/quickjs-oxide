//! Deterministic native-call stack budgeting.

use super::*;

impl Runtime {
    pub(super) fn native_call_would_overflow(&self, target: NativeFunctionId) -> bool {
        // Ordinary Function.prototype.call entries are tail-forwarded by
        // `call_internal`: each logical frame consumes one argument and no
        // Rust native frame remains around the target call. A native-stack
        // family ceiling would therefore reject valid QuickJS call chains
        // without protecting the host stack.
        if target == NativeFunctionId::FunctionPrototypeCall {
            return false;
        }

        // QuickJS checks its platform C-stack pointer before every native
        // call. Rust frame sizes do not map to that byte threshold, so keep a
        // deterministic call-entry ceiling on recursive native/callback paths.
        // Preserve a catchable JavaScript stack-overflow completion without
        // risking the host stack.
        let native_stack_weight = |target| match target {
            // Ordinary Function.prototype.call invocations are represented by
            // logical ActiveFrameGuards but tail-forwarded in one Rust frame.
            // Keep their diagnostic frames without double-charging the target
            // family's proven stack budget.
            NativeFunctionId::FunctionPrototypeCall => 0,
            NativeFunctionId::ArrayPrototypeJoin(_) | NativeFunctionId::ArrayPrototypeToString => {
                1_usize
            }
            NativeFunctionId::ArrayPrototypeSort | NativeFunctionId::ArrayPrototypeToSorted => 4,
            NativeFunctionId::ArrayPrototypeSlice(_)
            | NativeFunctionId::ArrayPrototypeToSpliced => 16,
            NativeFunctionId::ArrayPrototypeFlatten(_) => 9,
            NativeFunctionId::ObjectGroupBy
            | NativeFunctionId::ObjectKeys(_)
            | NativeFunctionId::ObjectGetOwnPropertyDescriptor
            | NativeFunctionId::ObjectHasOwn
            | NativeFunctionId::ObjectAssign
            | NativeFunctionId::PrimitiveConstructor(PrimitiveKind::String)
            | NativeFunctionId::StringStatic(_) => 8,
            // A key-coercion reentry retains the iterator, entry, result and
            // conversion stacks at once, making this family comparable to the
            // heaviest slice/splice native paths on a 2 MiB libtest thread.
            NativeFunctionId::ObjectFromEntries => 16,
            // Compile can re-enter through pattern/flags ToString. Its frames
            // are smaller than the RegExp Symbol protocol loops, but eight
            // nested calls are the proven-safe 2 MiB boundary.
            NativeFunctionId::RegExp(RegExpNativeKind::Compile) => 8,
            // The replace protocols alternate through user hooks, exec and
            // functional replacers. Nine nested protocol entries are required
            // by the pinned finite-recursion oracle; charge them like compile
            // while rejecting the tenth before the host stack is endangered.
            NativeFunctionId::StringPrototypeReplace(_)
            | NativeFunctionId::RegExp(RegExpNativeKind::Replace) => 8,
            // String receiver/argument conversion and RegExp protocol
            // callbacks retain native and property-call stacks while
            // recursively entering these methods.
            NativeFunctionId::StringPrototypeIncludes(_)
            | NativeFunctionId::StringPrototypeMatch
            | NativeFunctionId::StringPrototypeSearch
            | NativeFunctionId::StringPrototypeSplit
            | NativeFunctionId::StringPrototypeSubrange(_)
            | NativeFunctionId::StringPrototypeRepeat
            | NativeFunctionId::StringPrototypePad(_)
            | NativeFunctionId::StringPrototypeTrim(_)
            | NativeFunctionId::StringPrototypeCase(_)
            | NativeFunctionId::StringPrototypeCreateHtml(_)
            | NativeFunctionId::RegExp(RegExpNativeKind::Match)
            | NativeFunctionId::RegExp(RegExpNativeKind::Search)
            | NativeFunctionId::RegExp(RegExpNativeKind::Split) => 16,
            _ => 8,
        };
        let active_native_cost = self
            .0
            .state
            .borrow()
            .active_frames
            .iter()
            .filter_map(|frame| {
                let ActiveFrameKind::Native { target, .. } = frame.kind else {
                    return None;
                };
                Some(native_stack_weight(target))
            })
            .sum::<usize>();
        // A family-only ceiling can be bypassed by alternating different
        // callback-capable builtins. The weighted budget preserves the deeper
        // proven-safe join/sort chains while charging unclassified native
        // frames conservatively. It remains a deterministic approximation of
        // QuickJS's real platform-stack check until native calls are
        // trampolined.
        // Leave room for one leaf native operation (for example an iterator
        // `next`) at a family's proven-safe recursion ceiling.
        if active_native_cost.saturating_add(native_stack_weight(target)) > 80 {
            return true;
        }
        let limit = match target {
            NativeFunctionId::ArrayPrototypeJoin(_) | NativeFunctionId::ArrayPrototypeToString => {
                64
            }
            NativeFunctionId::ArrayPrototypeSort | NativeFunctionId::ArrayPrototypeToSorted => 16,
            NativeFunctionId::ArrayPrototypeSlice(_)
            | NativeFunctionId::ArrayPrototypeToSpliced => 4,
            NativeFunctionId::ArrayPrototypeFlatten(_) => 8,
            // Callback reentry retains the iterator and group-array building
            // stacks together. Reject the ninth family frame so the error can
            // still be allocated on the default libtest thread.
            NativeFunctionId::ObjectGroupBy => 8,
            // The heaviest measured getter-reentry path can exhaust a 2 MiB
            // host thread while entering the tenth family frame.
            NativeFunctionId::ObjectKeys(_) => 9,
            // ToPropertyKey may recursively re-enter through @@toPrimitive.
            NativeFunctionId::ObjectGetOwnPropertyDescriptor => 9,
            // This has the same key-coercion reentry shape as the descriptor
            // static; entering a tenth family frame can exhaust a 2 MiB
            // libtest thread before the general weighted budget rejects the
            // following call.
            NativeFunctionId::ObjectHasOwn => 9,
            NativeFunctionId::ObjectAssign => 9,
            NativeFunctionId::ObjectFromEntries => 4,
            NativeFunctionId::RegExp(RegExpNativeKind::Compile) => 8,
            NativeFunctionId::StringPrototypeReplace(_)
            | NativeFunctionId::RegExp(RegExpNativeKind::Replace) => 9,
            // Symbol protocols, receiver and argument conversions can alternate
            // between these String methods. Reject their shared fifth frame
            // while leaving weighted room for one callback leaf.
            NativeFunctionId::StringPrototypeIncludes(_)
            | NativeFunctionId::StringPrototypeMatch
            | NativeFunctionId::StringPrototypeSearch
            | NativeFunctionId::StringPrototypeSplit
            | NativeFunctionId::StringPrototypeSubrange(_)
            | NativeFunctionId::StringPrototypeRepeat
            | NativeFunctionId::StringPrototypePad(_)
            | NativeFunctionId::StringPrototypeTrim(_)
            | NativeFunctionId::StringPrototypeCase(_)
            | NativeFunctionId::StringPrototypeCreateHtml(_)
            | NativeFunctionId::RegExp(RegExpNativeKind::Match)
            | NativeFunctionId::RegExp(RegExpNativeKind::Search)
            | NativeFunctionId::RegExp(RegExpNativeKind::Split) => 4,
            // ToString, ToNumber and String.raw's property/conversion path can
            // all re-enter any other member of this constructor family.
            NativeFunctionId::PrimitiveConstructor(PrimitiveKind::String)
            | NativeFunctionId::StringStatic(_) => 9,
            _ => return false,
        };

        let in_family = |candidate| match target {
            NativeFunctionId::ArrayPrototypeJoin(_) | NativeFunctionId::ArrayPrototypeToString => {
                matches!(
                    candidate,
                    NativeFunctionId::ArrayPrototypeJoin(_)
                        | NativeFunctionId::ArrayPrototypeToString
                )
            }
            NativeFunctionId::ArrayPrototypeSort | NativeFunctionId::ArrayPrototypeToSorted => {
                matches!(
                    candidate,
                    NativeFunctionId::ArrayPrototypeSort | NativeFunctionId::ArrayPrototypeToSorted
                )
            }
            NativeFunctionId::ArrayPrototypeSlice(_)
            | NativeFunctionId::ArrayPrototypeToSpliced => {
                matches!(
                    candidate,
                    NativeFunctionId::ArrayPrototypeSlice(_)
                        | NativeFunctionId::ArrayPrototypeToSpliced
                )
            }
            NativeFunctionId::ArrayPrototypeFlatten(_) => {
                matches!(candidate, NativeFunctionId::ArrayPrototypeFlatten(_))
            }
            NativeFunctionId::ObjectGroupBy => {
                matches!(candidate, NativeFunctionId::ObjectGroupBy)
            }
            NativeFunctionId::ObjectKeys(_) => {
                matches!(candidate, NativeFunctionId::ObjectKeys(_))
            }
            NativeFunctionId::ObjectGetOwnPropertyDescriptor => {
                matches!(candidate, NativeFunctionId::ObjectGetOwnPropertyDescriptor)
            }
            NativeFunctionId::ObjectHasOwn => {
                matches!(candidate, NativeFunctionId::ObjectHasOwn)
            }
            NativeFunctionId::ObjectAssign => {
                matches!(candidate, NativeFunctionId::ObjectAssign)
            }
            NativeFunctionId::ObjectFromEntries => {
                matches!(candidate, NativeFunctionId::ObjectFromEntries)
            }
            NativeFunctionId::RegExp(RegExpNativeKind::Compile) => {
                matches!(
                    candidate,
                    NativeFunctionId::RegExp(RegExpNativeKind::Compile)
                )
            }
            NativeFunctionId::StringPrototypeReplace(_)
            | NativeFunctionId::RegExp(RegExpNativeKind::Replace) => matches!(
                candidate,
                NativeFunctionId::StringPrototypeReplace(_)
                    | NativeFunctionId::RegExp(RegExpNativeKind::Replace)
            ),
            NativeFunctionId::StringPrototypeIncludes(_)
            | NativeFunctionId::StringPrototypeMatch
            | NativeFunctionId::StringPrototypeSearch
            | NativeFunctionId::StringPrototypeSplit
            | NativeFunctionId::StringPrototypeSubrange(_)
            | NativeFunctionId::StringPrototypeRepeat
            | NativeFunctionId::StringPrototypePad(_)
            | NativeFunctionId::StringPrototypeTrim(_)
            | NativeFunctionId::StringPrototypeCase(_)
            | NativeFunctionId::StringPrototypeCreateHtml(_)
            | NativeFunctionId::RegExp(RegExpNativeKind::Match)
            | NativeFunctionId::RegExp(RegExpNativeKind::Search)
            | NativeFunctionId::RegExp(RegExpNativeKind::Split) => matches!(
                candidate,
                NativeFunctionId::StringPrototypeIncludes(_)
                    | NativeFunctionId::StringPrototypeMatch
                    | NativeFunctionId::StringPrototypeSearch
                    | NativeFunctionId::StringPrototypeSplit
                    | NativeFunctionId::StringPrototypeSubrange(_)
                    | NativeFunctionId::StringPrototypeRepeat
                    | NativeFunctionId::StringPrototypePad(_)
                    | NativeFunctionId::StringPrototypeTrim(_)
                    | NativeFunctionId::StringPrototypeCase(_)
                    | NativeFunctionId::StringPrototypeCreateHtml(_)
                    | NativeFunctionId::RegExp(RegExpNativeKind::Match)
                    | NativeFunctionId::RegExp(RegExpNativeKind::Search)
                    | NativeFunctionId::RegExp(RegExpNativeKind::Split)
            ),
            NativeFunctionId::PrimitiveConstructor(PrimitiveKind::String)
            | NativeFunctionId::StringStatic(_) => matches!(
                candidate,
                NativeFunctionId::PrimitiveConstructor(PrimitiveKind::String)
                    | NativeFunctionId::StringStatic(_)
            ),
            _ => false,
        };
        self.0
            .state
            .borrow()
            .active_frames
            .iter()
            .filter(|frame| {
                let ActiveFrameKind::Native { target, .. } = frame.kind else {
                    return false;
                };
                in_family(target)
            })
            .count()
            >= limit
    }
}
