//! The network's shared vocabulary: the spanning tree seen from a device, routes,
//! link hues and policies, and device names with their hotkey letter. Every map (the
//! hub, a device's LINKS, the edge editor, the send-to picker) is drawn by `nebula`.
use super::{fit, Cx};
use crate::ui::views::{Access, EdgeKind, LabDevice, LabEdge, Tok};
use ratatui::{
    buffer::Buffer,
    style::{Modifier, Style},
};

pub(super) struct Node<'a> {
    pub(super) d: &'a LabDevice,
    pub(super) hop: u8,
    /// The link to its parent (None for devices with no routed link).
    pub(super) up: Option<&'a LabEdge>,
    pub(super) parent: Option<usize>,
}

/// Jump edges are drawn through the hop they use, not as their own line.
fn routed(e: &LabEdge) -> bool {
    !matches!(e.kind, EdgeKind::Jump(_))
}

fn between<'a>(cx: &Cx<'a>, a: &str, b: &str) -> Option<&'a LabEdge> {
    cx.s.edges
        .iter()
        .find(|e| routed(e) && ((e.from == a && e.to == b) || (e.from == b && e.to == a)))
}

pub(super) fn mesh_tok(cx: &Cx, id: &str) -> Tok {
    mesh_tok_at(cx.s.meshes.iter().position(|m| m.id == id).unwrap_or(0))
}

/// The hue of the `i`th mesh, each its own family apart from teal (lan) and the
/// accent, in 16 colours too: violet, blue, amber, then pink (magenta, bright blue,
/// yellow, bright magenta), round again after four.
pub(super) fn mesh_tok_at(i: usize) -> Tok {
    [Tok::Mesh, Tok::Id(3), Tok::Amber, Tok::Id(5)][i % 4]
}

/// A link's hue: its mesh, teal for lan, neutral for direct ssh, red when down.
pub(super) fn edge_tok(cx: &Cx, e: &LabEdge) -> Tok {
    if e.up {
        hue_tok(cx, e)
    } else {
        Tok::Err
    }
}

/// The hue of a link's network, up or not.
fn hue_tok(cx: &Cx, e: &LabEdge) -> Tok {
    match &e.kind {
        EdgeKind::Mesh(m) => mesh_tok(cx, m),
        EdgeKind::Lan => Tok::Lan,
        EdgeKind::Direct | EdgeKind::Jump(_) => Tok::Muted,
    }
}

/// The routed path from the viewer to `to` (at most three hops, shortest first), as the
/// links taken; empty for the viewer itself or no route.
pub(super) fn route<'a>(cx: &Cx<'a>, to: &str) -> Vec<&'a LabEdge> {
    route_from(cx, &cx.s.viewer, to)
}

/// The same from any device: only links that are up and open in the direction taken.
pub(super) fn route_from<'a>(cx: &Cx<'a>, from: &str, to: &str) -> Vec<&'a LabEdge> {
    let s = cx.s;
    let mut paths: Vec<(String, Vec<&LabEdge>)> = vec![(from.to_string(), Vec::new())];
    let mut seen = vec![from.to_string()];
    for _ in 0..3 {
        let mut next = Vec::new();
        for (at, path) in &paths {
            for e in s.edges.iter().filter(|e| routed(e) && e.up) {
                let other = if &e.from == at && e.forward != Access::Denied {
                    &e.to
                } else if &e.to == at && e.reverse != Access::Denied {
                    &e.from
                } else {
                    continue;
                };
                if seen.contains(other) {
                    continue;
                }
                let mut p = path.clone();
                p.push(e);
                if other == to {
                    return p;
                }
                seen.push(other.clone());
                next.push((other.clone(), p));
            }
        }
        paths = next;
    }
    Vec::new()
}

/// Policy shorthand, the arrow pointing the way access is allowed from `from_left`'s
/// side. `short` keeps only the glyph.
pub(super) fn policy(cx: &Cx, e: &LabEdge, from_left: bool, short: bool) -> String {
    if !e.up {
        return if short {
            cx.g("✕", "!").into()
        } else {
            format!("{} down", cx.g("✕", "!"))
        };
    }
    let (to_right, to_left) = if from_left {
        (e.forward, e.reverse)
    } else {
        (e.reverse, e.forward)
    };
    let word = |a: Access| match a {
        Access::Key => "key",
        Access::Password => "pass",
        Access::Denied => "off",
    };
    let (g, w) = match (to_right, to_left) {
        (Access::Denied, Access::Denied) => (cx.g("⊘", "off"), "off".to_string()),
        (a, Access::Denied) => (cx.g("→", "->"), word(a).into()),
        (Access::Denied, a) => (cx.g("←", "<-"), word(a).into()),
        (a, b) if a == b => (cx.g("⇄", "<>"), word(a).into()),
        (a, b) => (cx.g("⇄", "<>"), format!("{}/{}", word(a), word(b))),
    };
    if short || g == w {
        g.into()
    } else {
        format!("{g} {w}")
    }
}

pub(super) fn closed(e: &LabEdge) -> bool {
    e.forward == Access::Denied && e.reverse == Access::Denied
}

// ------------------------------------------------------------------------ layout

/// Spanning tree from `root` over routed links, breadth first, up to `depth` hops.
/// On the hub (depth 3) devices with no routed link join the first column unlinked.
pub(super) fn tree<'a>(cx: &Cx<'a>, root: &str, depth: usize) -> Vec<Node<'a>> {
    let s = cx.s;
    let idx = |id: &str| s.devices.iter().position(|d| d.id == id).unwrap_or(0);
    let group = |id: &str| {
        s.meshes
            .iter()
            .position(|m| m.members.iter().any(|x| x == id))
            .unwrap_or(s.meshes.len())
    };
    let node = |d, hop, up, parent| Node { d, hop, up, parent };
    let mut nodes: Vec<Node> = Vec::new();
    let mut seen: Vec<&str> = vec![root];
    let mut frontier: Vec<Option<usize>> = vec![None];
    for hop in 1..=depth as u8 {
        let mut next = Vec::new();
        for &p in &frontier {
            let pid = p.map_or(root, |i| nodes[i].d.id.as_str());
            let mut kids: Vec<(&LabDevice, &LabEdge)> = s
                .devices
                .iter()
                .filter(|d| !seen.contains(&d.id.as_str()))
                .filter_map(|d| Some((d, between(cx, pid, &d.id)?)))
                .collect();
            // Mesh members sit together, so their trunk stays short.
            kids.sort_by_key(|(d, _)| (group(&d.id), idx(&d.id)));
            for (d, e) in kids {
                seen.push(&d.id);
                nodes.push(node(d, hop, Some(e), p));
                next.push(Some(nodes.len() - 1));
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    // A device reached by jumping through a relay hangs from that relay, its line the
    // jump link's (the hop itself is drawn once, as the relay's own link).
    for d in s.devices.iter().filter(|d| !seen.contains(&d.id.as_str())) {
        let jump = s.edges.iter().find_map(|e| match &e.kind {
            EdgeKind::Jump(via)
                if (e.from == root && e.to == d.id) || (e.to == root && e.from == d.id) =>
            {
                let p = nodes.iter().position(|n| n.d.id == *via)?;
                (usize::from(nodes[p].hop) < depth).then_some((p, e))
            }
            _ => None,
        });
        if let Some((p, e)) = jump {
            let hop = nodes[p].hop + 1;
            nodes.push(node(d, hop, Some(e), Some(p)));
        } else if depth >= 3 {
            nodes.push(node(d, 1, None, None));
        }
    }
    nodes
}

/// The name, with a keycap in front when the hotkey letter is not in it.
pub(super) fn has_letter(d: &LabDevice) -> Option<usize> {
    d.name
        .chars()
        .position(|c| c.eq_ignore_ascii_case(&d.hotkey))
}

/// One row of the phone's device tree.
pub(super) struct TreeRow<'a> {
    pub d: &'a LabDevice,
    /// The link to its parent (None for the root and unlinked devices).
    pub edge: Option<&'a LabEdge>,
    /// Per ancestor level below the root: whether that ancestor has later siblings.
    pub guides: Vec<bool>,
    pub last: bool,
}

/// The spanning tree as indented rows, depth first: the root, then each relay followed
/// by the devices reached through it.
pub(super) fn tree_rows<'a>(cx: &Cx<'a>, root: &str) -> Vec<TreeRow<'a>> {
    let nodes = tree(cx, root, 3);
    let mut out = Vec::new();
    if let Some(d) = cx.s.device(root) {
        out.push(TreeRow {
            d,
            edge: None,
            guides: Vec::new(),
            last: false,
        });
    }
    fn walk<'a>(
        nodes: &[Node<'a>],
        p: Option<usize>,
        guides: Vec<bool>,
        out: &mut Vec<TreeRow<'a>>,
    ) {
        let kids: Vec<usize> = (0..nodes.len()).filter(|&k| nodes[k].parent == p).collect();
        for (j, &k) in kids.iter().enumerate() {
            let last = j + 1 == kids.len();
            out.push(TreeRow {
                d: nodes[k].d,
                edge: nodes[k].up,
                guides: guides.clone(),
                last,
            });
            let mut g = guides.clone();
            g.push(!last);
            walk(nodes, Some(k), g, out);
        }
    }
    walk(&nodes, None, Vec::new(), &mut out);
    out
}

// ------------------------------------------------------------------------- draw

/// A device's name with its hotkey letter underlined in the accent (letters are keys);
/// where letters are not keys the name is entirely plain (no letter stands out, so
/// nothing suggests a key that does something else there). Returns columns written.
#[allow(clippy::too_many_arguments)]
pub(super) fn name(
    cx: &Cx,
    buf: &mut Buffer,
    x: u16,
    y: u16,
    d: &LabDevice,
    nmax: usize,
    st: Style,
    keys: bool,
) -> u16 {
    let nm = fit(&d.name, nmax, cx.uni());
    if !keys {
        return cx.put(buf, x, y, nmax as u16 + 1, &nm, st);
    }
    let mut w = 0;
    let at = has_letter(d).filter(|&i| i + 1 < nm.chars().count() || nm == d.name);
    let key = cx
        .st(Tok::Accent)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    // A letter the name lacks stands before it in the same key ink (never the
    // device's identity chip, whose hue can read as an error beside the graph).
    if at.is_none() {
        w += cx.put(buf, x, y, 1, &d.hotkey.to_string(), st.patch(key)) + 1;
    }
    for (i, ch) in nm.chars().enumerate() {
        // ASCII has no underline: the letter sits in parentheses, `(t)ranquility`.
        let s = if at == Some(i) && !cx.uni() {
            format!("({ch})")
        } else {
            ch.to_string()
        };
        let cst = if at == Some(i) { st.patch(key) } else { st };
        w += cx.put(buf, x + w, y, 3, &s, cst);
    }
    w
}

pub(super) fn name_w(cx: &Cx, d: &LabDevice, nmax: usize, keys: bool) -> u16 {
    let nm = fit(&d.name, nmax, cx.uni());
    let at = has_letter(d).filter(|&i| i + 1 < nm.chars().count() || nm == d.name);
    super::width(&nm, cx.uni())
        + match (at, keys) {
            (None, true) => 2,
            (Some(_), true) if !cx.uni() => 2,
            _ => 0,
        }
}
