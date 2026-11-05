//! Shared, incrementally maintained file-listing state.
//!
//! `Entries` is the listing itself: an `Arc`-shared vector with a revision, so the
//! navigation cache, restart snapshots and browser caches share one copy and every
//! mutation is visible to derived caches. `ViewIndex` holds what rendering and search
//! derive from it (lowercase names, the hidden/filter view, search matches, the largest
//! size). Pages appended in the helper's order extend these caches instead of rebuilding.
use super::Entry;
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

static REVISIONS: AtomicU64 = AtomicU64::new(1);
fn next_revision() -> u64 {
    REVISIONS.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
pub(super) struct Entries {
    items: Arc<Vec<Entry>>,
    revision: u64,
    /// Set by `append`: (revision, length) before the append, for incremental caches.
    appended_from: Option<(u64, usize)>,
}
impl Default for Entries {
    fn default() -> Self {
        Vec::new().into()
    }
}
impl From<Vec<Entry>> for Entries {
    fn from(items: Vec<Entry>) -> Self {
        Self {
            items: Arc::new(items),
            revision: next_revision(),
            appended_from: None,
        }
    }
}
impl FromIterator<Entry> for Entries {
    fn from_iter<I: IntoIterator<Item = Entry>>(iter: I) -> Self {
        iter.into_iter().collect::<Vec<_>>().into()
    }
}
impl std::ops::Deref for Entries {
    type Target = Vec<Entry>;
    fn deref(&self) -> &Vec<Entry> {
        &self.items
    }
}
/// Any mutable access is a new revision; derived caches rebuild on next use.
impl std::ops::DerefMut for Entries {
    fn deref_mut(&mut self) -> &mut Vec<Entry> {
        self.revision = next_revision();
        self.appended_from = None;
        Arc::make_mut(&mut self.items)
    }
}
impl serde::Serialize for Entries {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.items.as_slice().serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for Entries {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<Entry>::deserialize(deserializer).map(Into::into)
    }
}
impl Entries {
    /// Append a page that sorts after every existing entry (helper `order` replies).
    pub(super) fn append(&mut self, more: Vec<Entry>) {
        let before = (self.revision, self.items.len());
        Arc::make_mut(&mut self.items).extend(more);
        self.revision = next_revision();
        self.appended_from = Some(before);
    }
    pub(super) fn shared(&self) -> Arc<Vec<Entry>> {
        self.items.clone()
    }
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }
}

/// Matches of an in-list search: (visible row, character positions in the name).
pub(super) type RowMatches = Arc<Vec<(usize, Vec<usize>)>>;

#[derive(Clone, Default)]
pub(super) struct ViewIndex {
    /// Entries revision that `lower`, `largest` and `rows` describe.
    revision: u64,
    lower: Arc<Vec<Box<str>>>,
    largest: u64,
    /// Total bytes of the listing's regular files.
    bytes: u64,
    /// (show_hidden, lowercase filter) the rows were built for.
    view: Option<(bool, String)>,
    rows: Arc<Vec<u32>>,
    /// Bumped whenever `rows` changes, so search caches can key on it.
    rows_generation: u64,
    search: Option<(u64, String, RowMatches)>,
}

/// The visible (hidden/filter-applied) rows of a listing. Cheap to clone; it owns
/// shared references, so callers may mutate the browser while holding it.
#[derive(Clone)]
pub(super) struct VisibleRows {
    entries: Arc<Vec<Entry>>,
    rows: Arc<Vec<u32>>,
}
impl VisibleRows {
    pub(super) fn empty() -> Self {
        Self {
            entries: Arc::new(Vec::new()),
            rows: Arc::new(Vec::new()),
        }
    }
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub(super) fn get(&self, row: usize) -> Option<&Entry> {
        self.rows.get(row).map(|&i| &self.entries[i as usize])
    }
    pub(super) fn iter(&self) -> impl DoubleEndedIterator<Item = &Entry> + ExactSizeIterator {
        self.rows.iter().map(|&i| &self.entries[i as usize])
    }
    pub(super) fn position_of(&self, path: &str) -> Option<usize> {
        self.iter().position(|e| e.path == path)
    }
}
impl std::ops::Index<usize> for VisibleRows {
    type Output = Entry;
    fn index(&self, row: usize) -> &Entry {
        self.get(row).expect("visible row in range")
    }
}

fn shown(entry: &Entry, lower: &str, show_hidden: bool, filter: &str) -> bool {
    (show_hidden || !(entry.hidden || entry.name.starts_with('.')))
        && (filter.is_empty() || lower.contains(filter))
}
fn lowercase(entry: &Entry) -> Box<str> {
    entry.name.to_lowercase().into_boxed_str()
}

impl ViewIndex {
    /// Bring the index up to date with `entries` and the view settings. Appends in
    /// helper order extend every cache; anything else rebuilds once.
    fn update(&mut self, entries: &Entries, show_hidden: bool, filter: &str) {
        let filter = filter.to_lowercase();
        if self.revision != entries.revision() {
            let incremental = entries
                .appended_from
                .filter(|(revision, length)| {
                    *revision == self.revision && *length == self.lower.len()
                })
                .map(|(_, length)| length);
            match incremental {
                Some(from) => {
                    let lower = Arc::make_mut(&mut self.lower);
                    for entry in &entries[from..] {
                        lower.push(lowercase(entry));
                        if entry.kind == "file" {
                            self.largest = self.largest.max(entry.size);
                            self.bytes = self.bytes.saturating_add(entry.size);
                        }
                    }
                    if self.view.as_ref() == Some(&(show_hidden, filter.clone())) {
                        let rows = Arc::make_mut(&mut self.rows);
                        for (i, entry) in entries.iter().enumerate().skip(from) {
                            if shown(entry, &self.lower[i], show_hidden, &filter) {
                                rows.push(i as u32);
                            }
                        }
                        self.rows_generation += 1;
                    } else {
                        self.view = None;
                    }
                }
                None => {
                    self.lower = Arc::new(entries.iter().map(lowercase).collect());
                    let files = || entries.iter().filter(|e| e.kind == "file").map(|e| e.size);
                    self.largest = files().max().unwrap_or(0);
                    self.bytes = files().fold(0u64, u64::saturating_add);
                    self.view = None;
                }
            }
            self.revision = entries.revision();
        }
        if self.view.as_ref() != Some(&(show_hidden, filter.clone())) {
            self.rows = Arc::new(
                entries
                    .iter()
                    .enumerate()
                    .filter(|(i, e)| shown(e, &self.lower[*i], show_hidden, &filter))
                    .map(|(i, _)| i as u32)
                    .collect(),
            );
            self.rows_generation += 1;
            self.view = Some((show_hidden, filter));
        }
    }
    pub(super) fn visible(
        &mut self,
        entries: &Entries,
        show_hidden: bool,
        filter: &str,
    ) -> VisibleRows {
        self.update(entries, show_hidden, filter);
        VisibleRows {
            entries: entries.shared(),
            rows: self.rows.clone(),
        }
    }
    pub(super) fn largest(&mut self, entries: &Entries, show_hidden: bool, filter: &str) -> u64 {
        self.update(entries, show_hidden, filter);
        self.largest
    }
    /// Total bytes of regular files as of the last update.
    pub(super) fn bytes(&self) -> u64 {
        self.bytes
    }
    /// Search matches over the visible rows, computed once per (rows, query).
    pub(super) fn search(
        &mut self,
        entries: &Entries,
        show_hidden: bool,
        filter: &str,
        query: &str,
    ) -> RowMatches {
        self.update(entries, show_hidden, filter);
        if query.is_empty() {
            return Arc::new(Vec::new());
        }
        if let Some((generation, cached, matches)) = &self.search {
            if *generation == self.rows_generation && cached == query {
                return matches.clone();
            }
        }
        let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
        let mut matches = Vec::new();
        for (row, &index) in self.rows.iter().enumerate() {
            let name = super::safe_label(&entries[index as usize].name);
            for positions in super::preview::line_matches(name.chars(), &needle) {
                matches.push((row, positions));
            }
        }
        // One row may match several times; rendering and n/N use one entry per row.
        let mut merged: Vec<(usize, Vec<usize>)> = Vec::with_capacity(matches.len());
        for (row, positions) in matches {
            if let Some((_, last)) = merged.last_mut().filter(|(last, _)| *last == row) {
                last.extend(positions);
            } else {
                merged.push((row, positions));
            }
        }
        let matches = Arc::new(merged);
        self.search = Some((self.rows_generation, query.to_owned(), matches.clone()));
        matches
    }
}

/// Clipboard membership for one listing, built once per frame instead of a
/// linear scan per row.
pub(super) fn clipboard_paths(entries: &[Entry]) -> HashSet<&str> {
    entries.iter().map(|e| e.path.as_str()).collect()
}
