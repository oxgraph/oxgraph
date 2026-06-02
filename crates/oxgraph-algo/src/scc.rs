//! Strongly connected components over a directed topology view.
//!
//! [`strongly_connected_components`] runs Tarjan's algorithm iteratively (no
//! recursion, so deep graphs cannot overflow the stack) over the subgraph
//! induced by the caller-provided element set. Only edges whose target is also
//! in that set participate. The element set is supplied explicitly because the
//! topology traits expose no global element enumeration.

use alloc::{vec, vec::Vec};

use oxgraph_topology::{ElementIndex, ElementSuccessors, TopologyBase};

/// Sentinel index marking an element that Tarjan has not yet discovered.
const UNVISITED: usize = usize::MAX;

/// Borrowed induced-subgraph context: the view plus the membership marker and
/// its index bound, bundled so the Tarjan helpers stay within the argument
/// budget.
struct Induced<'view, G> {
    /// The topology view being traversed.
    graph: &'view G,
    /// Membership marker indexed by element index.
    member: &'view [bool],
    /// Exclusive index bound (`graph.element_bound()`).
    bound: usize,
}

/// One node's in-progress expansion on the explicit depth-first work stack.
struct Frame<Id> {
    /// Node whose successors this frame expands.
    node: Id,
    /// Member-filtered successors, materialized so expansion can resume by
    /// index after a child frame is pushed.
    successors: Vec<Id>,
    /// Index of the next successor to visit.
    position: usize,
}

/// Mutable Tarjan scratch shared across the per-start depth-first walks.
struct TarjanScratch<Id> {
    /// Discovery order assigned to each element index, or [`UNVISITED`].
    order_index: Vec<usize>,
    /// Lowest discovery order reachable from each element index.
    lowlink: Vec<usize>,
    /// Whether each element index is currently on the Tarjan stack.
    on_stack: Vec<bool>,
    /// The Tarjan stack of elements awaiting component assignment.
    stack: Vec<Id>,
    /// Completed components in discovery-completion order.
    components: Vec<Vec<Id>>,
    /// Monotonic discovery counter.
    counter: usize,
}

/// Collects the member-filtered successors of `node`.
///
/// # Performance
///
/// This function is `O(out-degree)`.
fn member_successors<G>(
    ctx: &Induced<'_, G>,
    node: <G as TopologyBase>::ElementId,
) -> Vec<<G as TopologyBase>::ElementId>
where
    G: ElementIndex + ElementSuccessors,
{
    ctx.graph
        .element_successors(node)
        .filter(|&successor| {
            let target = ctx.graph.element_index(successor);
            target < ctx.bound && ctx.member[target]
        })
        .collect()
}

/// Registers `node` as discovered and pushes its work frame.
///
/// # Performance
///
/// This function is `O(out-degree)`.
fn discover<G>(
    ctx: &Induced<'_, G>,
    node: <G as TopologyBase>::ElementId,
    scratch: &mut TarjanScratch<<G as TopologyBase>::ElementId>,
    work: &mut Vec<Frame<<G as TopologyBase>::ElementId>>,
) where
    G: ElementIndex + ElementSuccessors,
{
    let index = ctx.graph.element_index(node);
    scratch.order_index[index] = scratch.counter;
    scratch.lowlink[index] = scratch.counter;
    scratch.counter += 1;
    scratch.on_stack[index] = true;
    scratch.stack.push(node);
    work.push(Frame {
        node,
        successors: member_successors(ctx, node),
        position: 0,
    });
}

/// Visits one successor of `node`: discovers it when new, or relaxes `node`'s
/// lowlink when it is still on the stack.
///
/// # Performance
///
/// This function is `O(out-degree)` only when it discovers a new node.
fn advance<G>(
    ctx: &Induced<'_, G>,
    node: <G as TopologyBase>::ElementId,
    successor: <G as TopologyBase>::ElementId,
    scratch: &mut TarjanScratch<<G as TopologyBase>::ElementId>,
    work: &mut Vec<Frame<<G as TopologyBase>::ElementId>>,
) where
    G: ElementIndex + ElementSuccessors,
{
    let target = ctx.graph.element_index(successor);
    if scratch.order_index[target] == UNVISITED {
        discover(ctx, successor, scratch, work);
    } else if scratch.on_stack[target] {
        let node_index = ctx.graph.element_index(node);
        if scratch.order_index[target] < scratch.lowlink[node_index] {
            scratch.lowlink[node_index] = scratch.order_index[target];
        }
    }
}

/// Pops a finished component rooted at `root_index` off the Tarjan stack.
///
/// # Performance
///
/// This function is `O(component size)`.
fn pop_component<G>(
    ctx: &Induced<'_, G>,
    root_index: usize,
    scratch: &mut TarjanScratch<<G as TopologyBase>::ElementId>,
) where
    G: ElementIndex + ElementSuccessors,
{
    let mut component = Vec::new();
    while let Some(popped) = scratch.stack.pop() {
        let popped_index = ctx.graph.element_index(popped);
        scratch.on_stack[popped_index] = false;
        component.push(popped);
        if popped_index == root_index {
            break;
        }
    }
    scratch.components.push(component);
}

/// Completes `node`: emits its component when it is a root, then propagates its
/// lowlink to the parent frame.
///
/// # Performance
///
/// This function is `O(component size)` when `node` roots a component.
fn finish<G>(
    ctx: &Induced<'_, G>,
    node: <G as TopologyBase>::ElementId,
    scratch: &mut TarjanScratch<<G as TopologyBase>::ElementId>,
    work: &mut Vec<Frame<<G as TopologyBase>::ElementId>>,
) where
    G: ElementIndex + ElementSuccessors,
{
    let node_index = ctx.graph.element_index(node);
    if scratch.lowlink[node_index] == scratch.order_index[node_index] {
        pop_component(ctx, node_index, scratch);
    }
    work.pop();
    if let Some(parent) = work.last() {
        let parent_index = ctx.graph.element_index(parent.node);
        if scratch.lowlink[node_index] < scratch.lowlink[parent_index] {
            scratch.lowlink[parent_index] = scratch.lowlink[node_index];
        }
    }
}

/// Runs one iterative depth-first walk from `start`, assigning every element it
/// reaches to a component.
///
/// # Performance
///
/// This function is `O(reachable elements + inspected edges)`.
fn visit<G>(
    ctx: &Induced<'_, G>,
    start: <G as TopologyBase>::ElementId,
    scratch: &mut TarjanScratch<<G as TopologyBase>::ElementId>,
) where
    G: ElementIndex + ElementSuccessors,
{
    let mut work: Vec<Frame<G::ElementId>> = Vec::new();
    discover(ctx, start, scratch, &mut work);
    while let Some(top) = work.last_mut() {
        let node = top.node;
        if top.position < top.successors.len() {
            let successor = top.successors[top.position];
            top.position += 1;
            advance(ctx, node, successor, scratch, &mut work);
        } else {
            finish(ctx, node, scratch, &mut work);
        }
    }
}

/// Returns the strongly connected components of `elements`.
///
/// Duplicate input elements are de-duplicated. Each component lists its members
/// in Tarjan stack-pop order; components are returned in completion order. Every
/// requested element appears in exactly one component (a singleton when it
/// participates in no cycle).
///
/// # Performance
///
/// This function is `O(b + n + m)` for `b = graph.element_bound()`, `n`
/// requested elements, and `m` inspected outgoing entries; it allocates
/// `O(b + n + m)` (the work stack materializes member-filtered successors).
pub fn strongly_connected_components<G>(
    graph: &G,
    elements: &[<G as TopologyBase>::ElementId],
) -> Vec<Vec<<G as TopologyBase>::ElementId>>
where
    G: ElementIndex + ElementSuccessors,
{
    let bound = graph.element_bound();
    let mut member = vec![false; bound];
    let mut nodes: Vec<G::ElementId> = Vec::new();
    for &element in elements {
        let index = graph.element_index(element);
        if index < bound && !member[index] {
            member[index] = true;
            nodes.push(element);
        }
    }

    let ctx = Induced {
        graph,
        member: &member,
        bound,
    };
    let mut scratch = TarjanScratch {
        order_index: vec![UNVISITED; bound],
        lowlink: vec![0usize; bound],
        on_stack: vec![false; bound],
        stack: Vec::new(),
        components: Vec::new(),
        counter: 0,
    };

    for &start in &nodes {
        if scratch.order_index[graph.element_index(start)] == UNVISITED {
            visit(&ctx, start, &mut scratch);
        }
    }

    scratch.components
}
