//! QuickJS rqsort index choreography, with comparison replies separated from swaps.
use std::cmp::Ordering;

pub(in crate::engine::builtins) enum SortAction {
    Complete,
    Compare(usize, usize),
    Swap(usize, usize),
}
#[derive(Clone, Copy)]
enum Phase {
    Pop,
    Partition,
    MedianFirst,
    MedianSecond(bool),
    MedianThird(bool),
    Pivot,
    Left,
    LeftReply,
    Right,
    RightReply,
    Cross,
    LowerSwap,
    UpperSwap,
    Split,
    Insertion,
    InsertionReply,
    InsertionSwap,
    HeapBuild,
    HeapExtract,
    Sift,
    SiftChild,
    SiftRoot,
    SiftSwap,
}
pub(in crate::engine::builtins) struct SortMachine {
    stack: [(usize, usize, usize); 50],
    stack_len: usize,
    base: usize,
    count: usize,
    depth: usize,
    phase: Phase,
    pivot: usize,
    scanned: usize,
    lower_equal: usize,
    left: usize,
    lower_equal_end: usize,
    upper_equal: usize,
    right: usize,
    upper_equal_start: usize,
    span: usize,
    offset: usize,
    lower_count: usize,
    upper_base: usize,
    upper_count: usize,
    current: usize,
    position: usize,
    heap_building: bool,
    heap_cursor: usize,
    heap_end: usize,
    root: usize,
    child: usize,
}
impl SortMachine {
    pub(in crate::engine::builtins) fn new(length: usize) -> Self {
        let mut stack = [(0, 0, 0); 50];
        stack[0] = (0, length, 0);
        Self {
            stack,
            stack_len: usize::from(length >= 2),
            base: 0,
            count: 0,
            depth: 0,
            phase: Phase::Pop,
            pivot: 0,
            scanned: 0,
            lower_equal: 0,
            left: 0,
            lower_equal_end: 0,
            upper_equal: 0,
            right: 0,
            upper_equal_start: 0,
            span: 0,
            offset: 0,
            lower_count: 0,
            upper_base: 0,
            upper_count: 0,
            current: 0,
            position: 0,
            heap_building: false,
            heap_cursor: 0,
            heap_end: 0,
            root: 0,
            child: 0,
        }
    }
    pub(in crate::engine::builtins) fn advance(
        &mut self,
        mut reply: Option<Ordering>,
    ) -> SortAction {
        loop {
            match self.phase {
                Phase::Pop => {
                    if self.stack_len == 0 {
                        return SortAction::Complete;
                    }
                    self.stack_len -= 1;
                    (self.base, self.count, self.depth) = self.stack[self.stack_len];
                    self.phase = Phase::Partition;
                }
                Phase::Partition => {
                    if self.count <= 6 {
                        self.current = self.base + 1;
                        self.position = self.current;
                        self.phase = Phase::Insertion;
                        continue;
                    }
                    self.depth += 1;
                    if self.depth > 50 {
                        self.heap_building = true;
                        self.heap_cursor = self.count / 2;
                        self.heap_end = self.count;
                        self.phase = Phase::HeapBuild;
                        continue;
                    }
                    let quarter = self.count >> 2;
                    self.phase = Phase::MedianFirst;
                    return SortAction::Compare(self.base + quarter, self.base + 2 * quarter);
                }
                Phase::MedianFirst => {
                    let less = reply.take().expect("rqsort comparison reply").is_lt();
                    self.phase = Phase::MedianSecond(less);
                    let quarter = self.count >> 2;
                    return SortAction::Compare(self.base + 2 * quarter, self.base + 3 * quarter);
                }
                Phase::MedianSecond(less) => {
                    let order = reply.take().expect("rqsort comparison reply");
                    let quarter = self.count >> 2;
                    if (less && order.is_lt()) || (!less && order.is_gt()) {
                        self.pivot = self.base + 2 * quarter;
                        self.phase = Phase::Pivot;
                    } else {
                        self.phase = Phase::MedianThird(less);
                        return SortAction::Compare(self.base + quarter, self.base + 3 * quarter);
                    }
                }
                Phase::MedianThird(less) => {
                    let order = reply.take().expect("rqsort comparison reply");
                    let quarter = self.count >> 2;
                    self.pivot = self.base
                        + if order.is_lt() == less {
                            3 * quarter
                        } else {
                            quarter
                        };
                    self.phase = Phase::Pivot;
                }
                Phase::Pivot => {
                    self.scanned = 1;
                    self.lower_equal = 1;
                    self.left = self.base + 1;
                    self.lower_equal_end = self.left;
                    self.upper_equal = self.count;
                    self.right = self.base + self.count;
                    self.upper_equal_start = self.right;
                    self.phase = Phase::Left;
                    return SortAction::Swap(self.base, self.pivot);
                }
                Phase::Left => {
                    if self.left < self.right {
                        self.phase = Phase::LeftReply;
                        return SortAction::Compare(self.base, self.left);
                    }
                    self.phase = Phase::Right;
                }
                Phase::LeftReply => {
                    let order = reply.take().expect("rqsort comparison reply");
                    if order.is_lt() {
                        self.phase = Phase::Right;
                        continue;
                    }
                    self.scanned += 1;
                    let left = self.left;
                    self.left += 1;
                    self.phase = Phase::Left;
                    if order.is_eq() {
                        let equal = self.lower_equal_end;
                        self.lower_equal += 1;
                        self.lower_equal_end += 1;
                        return SortAction::Swap(equal, left);
                    }
                }
                Phase::Right => {
                    self.right -= 1;
                    if self.left >= self.right {
                        self.phase = Phase::Cross;
                        continue;
                    }
                    self.phase = Phase::RightReply;
                    return SortAction::Compare(self.base, self.right);
                }
                Phase::RightReply => {
                    let order = reply.take().expect("rqsort comparison reply");
                    if order.is_gt() {
                        self.phase = Phase::Cross;
                        continue;
                    }
                    self.phase = Phase::Right;
                    if order.is_eq() {
                        self.upper_equal -= 1;
                        self.upper_equal_start -= 1;
                        return SortAction::Swap(self.upper_equal_start, self.right);
                    }
                }
                Phase::Cross => {
                    if self.left < self.right {
                        let left = self.left;
                        self.scanned += 1;
                        self.left += 1;
                        self.phase = Phase::Left;
                        return SortAction::Swap(left, self.right);
                    }
                    self.span =
                        (self.lower_equal_end - self.base).min(self.left - self.lower_equal_end);
                    self.lower_count = self.scanned - self.lower_equal;
                    self.offset = 0;
                    self.phase = Phase::LowerSwap;
                }
                Phase::LowerSwap => {
                    if self.offset < self.span {
                        let offset = self.offset;
                        self.offset += 1;
                        return SortAction::Swap(
                            self.base + offset,
                            self.left - self.span + offset,
                        );
                    }
                    let top = self.base + self.count;
                    let middle = self.upper_equal_start - self.left;
                    self.upper_base = top - middle;
                    self.upper_count = self.upper_equal - self.scanned;
                    self.span = (top - self.upper_equal_start).min(middle);
                    self.offset = 0;
                    self.phase = Phase::UpperSwap;
                }
                Phase::UpperSwap => {
                    if self.offset < self.span {
                        let offset = self.offset;
                        self.offset += 1;
                        return SortAction::Swap(
                            self.left + offset,
                            self.base + self.count - self.span + offset,
                        );
                    }
                    self.phase = Phase::Split;
                }
                Phase::Split => {
                    debug_assert!(self.stack_len < self.stack.len());
                    if self.lower_count > self.upper_count {
                        self.stack[self.stack_len] = (self.base, self.lower_count, self.depth);
                        self.base = self.upper_base;
                        self.count = self.upper_count;
                    } else {
                        self.stack[self.stack_len] =
                            (self.upper_base, self.upper_count, self.depth);
                        self.count = self.lower_count;
                    }
                    self.stack_len += 1;
                    self.phase = Phase::Partition;
                }
                Phase::Insertion => {
                    if self.current >= self.base + self.count {
                        self.phase = Phase::Pop;
                        continue;
                    }
                    if self.position <= self.base {
                        self.current += 1;
                        self.position = self.current;
                        continue;
                    }
                    self.phase = Phase::InsertionReply;
                    return SortAction::Compare(self.position - 1, self.position);
                }
                Phase::InsertionReply => {
                    if reply.take().expect("rqsort comparison reply").is_gt() {
                        self.phase = Phase::InsertionSwap;
                    } else {
                        self.current += 1;
                        self.position = self.current;
                        self.phase = Phase::Insertion;
                    }
                }
                Phase::InsertionSwap => {
                    let position = self.position;
                    self.position -= 1;
                    self.phase = Phase::Insertion;
                    return SortAction::Swap(position, position - 1);
                }
                Phase::HeapBuild => {
                    if self.heap_cursor == 0 {
                        self.heap_building = false;
                        self.heap_cursor = self.count - 1;
                        self.phase = Phase::HeapExtract;
                        continue;
                    }
                    self.heap_cursor -= 1;
                    self.root = self.heap_cursor;
                    self.phase = Phase::Sift;
                }
                Phase::HeapExtract => {
                    if self.heap_cursor == 0 {
                        self.phase = Phase::Pop;
                        continue;
                    }
                    self.heap_end = self.heap_cursor;
                    self.heap_cursor -= 1;
                    self.root = 0;
                    self.phase = Phase::Sift;
                    return SortAction::Swap(self.base, self.base + self.heap_end);
                }
                Phase::Sift => {
                    self.child = self.root * 2 + 1;
                    if self.child >= self.heap_end {
                        self.phase = if self.heap_building {
                            Phase::HeapBuild
                        } else {
                            Phase::HeapExtract
                        };
                        continue;
                    }
                    if self.child + 1 < self.heap_end {
                        self.phase = Phase::SiftChild;
                        return SortAction::Compare(
                            self.base + self.child,
                            self.base + self.child + 1,
                        );
                    }
                    self.phase = Phase::SiftRoot;
                    return SortAction::Compare(self.base + self.root, self.base + self.child);
                }
                Phase::SiftChild => {
                    if !reply.take().expect("rqsort comparison reply").is_gt() {
                        self.child += 1;
                    }
                    self.phase = Phase::SiftRoot;
                    return SortAction::Compare(self.base + self.root, self.base + self.child);
                }
                Phase::SiftRoot => {
                    if reply.take().expect("rqsort comparison reply").is_gt() {
                        self.phase = if self.heap_building {
                            Phase::HeapBuild
                        } else {
                            Phase::HeapExtract
                        };
                    } else {
                        self.phase = Phase::SiftSwap;
                    }
                }
                Phase::SiftSwap => {
                    let root = self.root;
                    self.root = self.child;
                    self.phase = Phase::Sift;
                    return SortAction::Swap(self.base + root, self.base + self.child);
                }
            }
        }
    }
}
