//! Active-panel adapters: keyboard and mouse motions share the same selection effects.
use super::navigation::{List, Target};
use super::{
    browser_entries, preview_display_lines, preview_max_scroll, shift, sidebar_actions, App, Focus,
    View,
};

pub(super) struct Active<'a> {
    app: &'a mut App,
}
pub(super) fn active(app: &mut App) -> Active<'_> {
    Active { app }
}
impl Target for Active<'_> {
    fn move_by(&mut self, delta: isize) {
        let app = &mut *self.app;
        if app.view == View::Containers && app.focus == Focus::Workspace {
            let length = app.container_rows().len();
            List {
                selected: &mut app.container_selected,
                length,
            }
            .move_by(delta);
            return;
        }
        if app.view == View::Network {
            app.network_detail_scroll = 0;
        }
        match app.focus {
            Focus::Devices => Devices { app }.move_by(delta),
            Focus::Actions => {
                let length = sidebar_actions(app).len();
                List {
                    selected: &mut app.side_selected,
                    length,
                }
                .move_by(delta);
            }
            Focus::Workspace => match app.view {
                View::Files => Files { app }.move_by(delta),
                View::Network => {
                    let length = app.network_rows().len();
                    List {
                        selected: &mut app.network_selected,
                        length,
                    }
                    .move_by(delta);
                }
                _ => {
                    let length = app.session_rows().len();
                    List {
                        selected: &mut app.selected,
                        length,
                    }
                    .move_by(delta);
                }
            },
        }
    }
}
struct Devices<'a> {
    app: &'a mut App,
}
impl Target for Devices<'_> {
    fn move_by(&mut self, delta: isize) {
        let network_selection = if self.app.view == View::Network {
            self.app.network_selection()
        } else {
            None
        };
        let rows = self.app.device_rows();
        let current = rows.iter().position(|d| *d == self.app.device).unwrap_or(0);
        self.app.device = rows[shift(current, delta, rows.len())];
        self.app.selected = 0;
        if self.app.view == View::Files {
            self.app.select_file_device();
        } else {
            self.app.refresh();
            if self.app.view == View::Network {
                self.app.restore_network_selection(network_selection);
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
                        (i64::from(current) + delta as i64).clamp(1, i64::from(max)) as u32;
                    if requested != current {
                        b.preview_requested_page = requested;
                        self.app.start_pdf_page_request();
                    }
                    return;
                }
                let (width, height) = b.preview_viewport.get();
                let lines = preview_display_lines(b, b.preview.as_deref().unwrap_or(""));
                let max = preview_max_scroll(&lines, width.max(1), height.max(1));
                b.preview_scroll.set(
                    (i64::from(b.preview_scroll.get()) + delta as i64).clamp(0, i64::from(max))
                        as u16,
                );
                b.preview_find.reveal.set(false);
                return;
            }
        }

        let len = self.app.visible_entries().len();
        if let Some(b) = &mut self.app.browser {
            b.restore_selection = None;
            b.selected = shift(b.selected, delta, len);
            if let Some(anchor) = b.visual_anchor {
                b.marked = b.visual_base.clone();
                let entries = browser_entries(b);
                let start = anchor.min(b.selected);
                let end = anchor.max(b.selected);
                for entry in entries.iter().skip(start).take(end - start + 1) {
                    if b.marked.len() < 256 {
                        b.marked.insert(entry.path.clone());
                    }
                }
            }
        }
    }
}
