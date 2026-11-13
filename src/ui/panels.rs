//! Active-panel adapters: keyboard and mouse motions share the same selection effects.
use super::navigation::{List, Target};
use super::{doc_max_scroll, shift, App, View};

pub(super) struct Active<'a> {
    app: &'a mut App,
}
pub(super) fn active(app: &mut App) -> Active<'_> {
    Active { app }
}
impl Target for Active<'_> {
    fn move_by(&mut self, delta: isize) {
        let app = &mut *self.app;
        if app.view == View::Containers {
            let length = app.container_rows().len();
            List {
                selected: &mut app.container_selected,
                length,
            }
            .move_by(delta);
            return;
        }
        match app.view {
            View::Files => Files { app }.move_by(delta),
            // The design's screens: the hub's zones, a device's slots, secret cards.
            View::Network | View::Secrets => app.screen_move(delta),
            View::Work if app.device > 0 => app.screen_move(delta),
            _ => {
                let length = app.session_rows().len();
                List {
                    selected: &mut app.selected,
                    length,
                }
                .move_by(delta);
            }
        }
    }
}
struct Files<'a> {
    app: &'a mut App,
}
impl Target for Files<'_> {
    fn move_by(&mut self, delta: isize) {
        if let Some(b) = &mut self.app.browser {
            if b.preview.is_some() {
                if b.preview_rich.as_ref().is_some_and(|p| p.kind == "pdf") {
                    let max = b
                        .preview_rich
                        .as_ref()
                        .and_then(|p| p.pages)
                        .unwrap_or(10000);
                    let current = b.preview_requested_page.max(1);
                    let requested =
                        (i64::from(current) + delta as i64).clamp(1, i64::from(max.max(1))) as u32;
                    if requested != current {
                        b.preview_requested_page = requested;
                        self.app.start_pdf_page_request();
                    }
                    return;
                }
                // The retained layout knows the wrapped row count; nothing re-wraps here.
                let max = doc_max_scroll(b);
                b.preview_scroll.set(
                    (i64::from(b.preview_scroll.get()) + delta as i64).clamp(0, i64::from(max))
                        as u32,
                );
                b.preview_find.reveal.set(false);
                // End follows a streaming preview to its end; nearer scrolling loads ahead.
                if let Some(stream) = &mut b.stream {
                    stream.follow = delta == super::navigation::END;
                }
                self.app.pump_preview_stream();
                return;
            }
        }

        let len = self.app.visible_entries().len();
        if let Some(b) = &mut self.app.browser {
            b.restore_selection = None;
            b.selected = shift(b.selected, delta, len);
            if let Some(anchor) = b.visual_anchor {
                b.marked = b.visual_base.clone();
                let entries = super::browser_entries(b);
                let start = anchor.min(b.selected);
                let end = anchor.max(b.selected);
                // `V` (yazi's unset mode) clears the range instead.
                let unset = self.app.yz.unset;
                for entry in entries.iter().skip(start).take(end - start + 1) {
                    if unset {
                        b.marked.remove(&entry.path);
                    } else if b.marked.len() < 4096 {
                        b.marked.insert(entry.path.clone());
                    }
                }
            }
        }
    }
}
