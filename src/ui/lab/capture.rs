//! `lab_capture`: env-driven dump of (view x size x variant x fixture) for one concept.
//! Run through `tests/lab.sh`, which also turns the dumps into PNGs and contact sheets.
use super::{caps::Pane, concepts, dump, render_frame, LabCaps, LabState, LabView};
use serde_json::json;
use std::{env, fs, path::PathBuf};

const SIZES: &str = "40x20,60x30,80x24,120x40,200x60";
const VARIANTS: &str = "dark-truecolor,light-truecolor,dark-16,light-16,ascii";

fn var(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn list(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_size(s: &str) -> (u16, u16) {
    let (w, h) = s
        .split_once('x')
        .unwrap_or_else(|| panic!("size `{s}` is not WxH"));
    let w: u16 = w.parse().expect("width");
    let h: u16 = h.parse().expect("height");
    assert!(
        w > 0 && h > 0 && w <= 500 && h <= 200,
        "size `{s}` out of range"
    );
    (w, h)
}

fn parse_pane(s: &str) -> Option<Pane> {
    Some(match s {
        "graph" => Pane::Graph,
        "rail" => Pane::Rail,
        "sessions" => Pane::Sessions,
        "preview" => Pane::Preview,
        "files" => Pane::Files,
        "inspector" => Pane::Inspector,
        "palette" => Pane::Palette,
        "dialog" => Pane::Dialog,
        "" | "auto" => return None,
        other => Pane::Custom(Box::leak(other.to_string().into_boxed_str())),
    })
}

#[test]
#[ignore = "render lab capture; run via tests/lab.sh"]
fn lab_capture() {
    let all = concepts::all();
    let names: Vec<&str> = all.iter().map(|c| c.name()).collect();
    let wanted = var("CX_LAB_CONCEPT", "");
    let concept = all
        .iter()
        .find(|c| c.name() == wanted)
        .unwrap_or_else(|| panic!("CX_LAB_CONCEPT `{wanted}` unknown; registered: {names:?}"));
    let out = PathBuf::from(env::var("CX_LAB_OUT").expect("CX_LAB_OUT output directory"));
    fs::create_dir_all(&out).expect("create CX_LAB_OUT");

    let sizes: Vec<(u16, u16)> = list(&var("CX_LAB_SIZES", SIZES))
        .iter()
        .map(|s| parse_size(s))
        .collect();
    let variants = list(&var("CX_LAB_VARIANTS", VARIANTS));
    let fixtures = list(&var("CX_LAB_FIXTURES", "typical"));
    let views_spec = var("CX_LAB_VIEWS", "all");
    let focus = parse_pane(&var("CX_LAB_FOCUS", ""));
    let selected: usize = var("CX_LAB_SELECTED", "0")
        .parse()
        .expect("CX_LAB_SELECTED");
    let frame: u32 = var("CX_LAB_FRAME", "0").parse().expect("CX_LAB_FRAME");
    let hover = env::var("CX_LAB_HOVER").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    });

    let mut entries = Vec::new();
    let mut warnings = 0;
    for fx in &fixtures {
        let state = LabState::fixture(fx)
            .unwrap_or_else(|| panic!("fixture `{fx}` unknown; use small, typical, large"));
        let views: Vec<LabView> = if views_spec == "all" {
            LabView::defaults(&state)
        } else {
            list(&views_spec)
                .iter()
                .map(|v| LabView::parse(v, &state).unwrap_or_else(|| panic!("view `{v}` unknown")))
                .collect()
        };
        for variant in &variants {
            let mut caps = LabCaps::from_variant(variant)
                .unwrap_or_else(|| panic!("variant `{variant}` unknown"));
            caps.focus = focus;
            caps.selected = selected;
            caps.frame = frame;
            caps.hover = hover;
            for view in &views {
                for &(w, h) in &sizes {
                    let buf = render_frame(concept.as_ref(), view, &state, &caps, w, h);
                    let (rows, lint) = dump::dump(&buf, &caps);
                    let targets: Vec<_> = concept
                        .targets(view, &state, &caps, buf.area)
                        .into_iter()
                        .map(|t| {
                            json!({"x": t.area.x, "y": t.area.y, "w": t.area.width,
                                   "h": t.area.height, "action": t.action})
                        })
                        .collect();
                    let mut stem = format!("{}__{w}x{h}__{variant}", view.slug());
                    if fixtures.len() > 1 || fx != "typical" {
                        stem.push_str(&format!("__{fx}"));
                    }
                    if lint.non_ascii > 0 || lint.color_leaks > 0 {
                        warnings += 1;
                        eprintln!(
                            "lint {stem}: non_ascii={} color_leaks={} {:?}",
                            lint.non_ascii, lint.color_leaks, lint.samples
                        );
                    }
                    let lint_json = json!({"non_ascii": lint.non_ascii,
                        "color_leaks": lint.color_leaks, "samples": lint.samples});
                    let doc = json!({
                        "concept": concept.name(), "view": view.slug(), "cols": w, "rows": h,
                        "variant": variant, "fixture": fx,
                        "theme": if caps.theme == super::Theme::Dark { "dark" } else { "light" },
                        "color": format!("{:?}", caps.color), "unicode": caps.unicode,
                        "lint": lint_json, "targets": targets, "cells": rows,
                    });
                    fs::write(out.join(format!("{stem}.json")), doc.to_string()).unwrap();
                    fs::write(out.join(format!("{stem}.txt")), dump::text(&buf)).unwrap();
                    entries.push(
                        json!({"stem": stem, "view": view.slug(), "cols": w, "rows": h,
                        "variant": variant, "fixture": fx, "lint": lint_json}),
                    );
                }
            }
        }
    }
    let manifest = json!({
        "concept": concept.name(), "summary": concept.summary(),
        "sizes": sizes.iter().map(|(w, h)| format!("{w}x{h}")).collect::<Vec<_>>(),
        "variants": variants, "fixtures": fixtures, "entries": entries,
    });
    fs::write(out.join("manifest.json"), manifest.to_string()).unwrap();
    eprintln!(
        "lab_capture: {} frames of `{}` -> {} ({warnings} with lint warnings)",
        entries.len(),
        concept.name(),
        out.display()
    );
}
