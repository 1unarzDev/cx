//! Concept registry. Add `mod <name>;` and one line in `all()`.
use super::Concept;

mod baseline;
mod glyphs;

pub fn all() -> Vec<Box<dyn Concept>> {
    vec![Box::new(baseline::Baseline), Box::new(glyphs::Glyphs)]
}
