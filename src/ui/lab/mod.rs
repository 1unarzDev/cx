//! Render lab: the production view layer (`ui::views`) captured from fixtures and
//! dumped (cells + styles) for `tests/lab_png.py`. Test-only; see docs/design/lab.md.
#![allow(dead_code)]

mod capture;
pub mod dump;

pub use super::views::*;

use ratatui::{buffer::Buffer, layout::Rect};

/// The registered concepts (the production design only).
pub fn concepts() -> Vec<Box<dyn Concept>> {
    vec![Box::new(mission::Mission)]
}

/// Render one frame of `concept` exactly as a capture does (used by tests and capture).
pub fn render_frame(
    concept: &dyn Concept,
    view: &LabView,
    state: &LabState,
    caps: &LabCaps,
    w: u16,
    h: u16,
) -> Buffer {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    concept.render(view.clone(), state, caps, area, &mut buf);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_specs_round_trip() {
        let s = state::typical();
        for v in LabView::defaults(&s) {
            let slug = v.slug();
            assert!(!slug.is_empty());
        }
        assert_eq!(
            LabView::parse("device:nas", &s),
            Some(LabView::DeviceFocus("nas".into()))
        );
        assert_eq!(
            LabView::parse("edge", &s),
            Some(LabView::EdgeEditor("tranquility-innovation".into()))
        );
        assert!(LabView::parse("nope", &s).is_none());
    }

    /// Every registered concept renders every default view, size, variant and fixture
    /// without panicking (sizes include degenerate ones).
    #[test]
    fn all_concepts_render_everywhere() {
        let sizes = [(1, 1), (10, 4), (40, 20), (200, 60)];
        for fx in FIXTURES {
            let s = LabState::fixture(fx).unwrap();
            for c in concepts() {
                for v in LabView::defaults(&s) {
                    for variant in ["light-256", "ascii"] {
                        let caps = LabCaps::from_variant(variant).unwrap();
                        for (w, h) in sizes {
                            let buf = render_frame(c.as_ref(), &v, &s, &caps, w, h);
                            if w == 40 {
                                let _ = dump::dump(&buf, &caps);
                            }
                        }
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod map_tests {
    use super::*;
    /// A network whose drawing is one row (the viewer and its direct peers) still
    /// draws: the map's band was one row, under the three it needs.
    #[test]
    fn a_one_row_network_still_draws_its_map() {
        let mut s = state::small();
        s.edges
            .retain(|e| e.from == "tranquility" && e.to == "innovation");
        let buf = render_frame(
            &mission::Mission,
            &LabView::Hub,
            &s,
            &LabCaps::default(),
            120,
            40,
        );
        let row = (2..20)
            .map(|y| {
                (0..120)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .find(|r| r.contains("tranquility"));
        let row = row.expect("map row");
        assert!(row.contains("innovation") && row.contains("nas"), "{row}");
    }
}
