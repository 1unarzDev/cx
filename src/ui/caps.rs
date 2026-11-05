//! Terminal capability detection (design spec section 9).
//!
//! Detection is a pure function of an environment lookup and a lazy probe, so
//! the whole matrix is table-tested. The live result is computed once per
//! process ([`caps`]) and never re-reads the environment afterwards. Probes are
//! bounded (infocmp 100 ms, tmux 200 ms) and run only when the environment
//! alone cannot decide; they never print or log environment values.
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

/// Colors the terminal can show. Ordered: a higher depth can show every lower one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ColorDepth {
    /// `NO_COLOR` or `TERM=dumb`: attributes only.
    None,
    Ansi16,
    Ansi256,
    TrueColor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Background {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Caps {
    pub color: ColorDepth,
    /// False selects the ASCII glyph set (borders included).
    pub unicode: bool,
    pub background: Background,
    pub tmux: bool,
    pub termux: bool,
    /// Termux or Blink: prefer the XS layout, larger chips and the tap bar.
    pub phone: bool,
    /// OSC 8 hyperlinks on the allowlist (never inside tmux).
    pub hyperlinks: bool,
    /// Heuristic only; icons still need `CX_ICONS=1` ([`Caps::icons`]).
    pub nerd_font_likely: bool,
    pub icons: bool,
}

impl Default for Caps {
    /// The deterministic profile used when nothing is known: truecolor, dark, Unicode.
    fn default() -> Self {
        Self {
            color: ColorDepth::TrueColor,
            unicode: true,
            background: Background::Dark,
            tmux: false,
            termux: false,
            phone: false,
            hyperlinks: false,
            nerd_font_likely: false,
            icons: false,
        }
    }
}

/// Terminfo facts for `$TERM`, when a probe could read them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Terminfo {
    pub colors: Option<u32>,
    /// `Tc` or `RGB` extended capability.
    pub rgb: bool,
}

/// Lazy side-channel facts. Implementations must be bounded and read-only.
pub(crate) trait Probe {
    fn terminfo(&self, _term: &str) -> Option<Terminfo> {
        None
    }
    /// `Some(true)` when the enclosing tmux client reports the `RGB` feature.
    fn tmux_rgb(&self) -> Option<bool> {
        None
    }
}

/// A probe that knows nothing; detection then uses the environment alone.
pub(crate) struct NoProbe;
impl Probe for NoProbe {}

impl Caps {
    /// Detect from an environment lookup (`None` = unset) and a probe.
    pub(crate) fn detect(env: &dyn Fn(&str) -> Option<String>, probe: &dyn Probe) -> Self {
        let get = |k: &str| env(k);
        let set = |k: &str| env(k).is_some();
        let nonempty = |k: &str| env(k).filter(|v| !v.is_empty());
        let term = get("TERM").unwrap_or_default().to_ascii_lowercase();
        let program = get("TERM_PROGRAM").unwrap_or_default();
        let lc_terminal = get("LC_TERMINAL").unwrap_or_default();
        let colorterm = get("COLORTERM").unwrap_or_default().to_ascii_lowercase();
        let tmux = nonempty("TMUX").is_some();
        let termux =
            set("TERMUX_VERSION") || get("PREFIX").is_some_and(|p| p.contains("com.termux"));
        let blink = program.eq_ignore_ascii_case("Blink") || lc_terminal == "Blink";
        let kitty = set("KITTY_WINDOW_ID") || term == "xterm-kitty";
        let ghostty = program.eq_ignore_ascii_case("ghostty")
            || set("GHOSTTY_RESOURCES_DIR")
            || term == "xterm-ghostty";
        let foot = term.starts_with("foot");
        let alacritty =
            set("ALACRITTY_SOCKET") || set("ALACRITTY_WINDOW_ID") || term == "alacritty";
        let wezterm = program == "WezTerm";
        let iterm = program == "iTerm.app";
        let vscode = program == "vscode";
        let dumb = term == "dumb";
        let console = term == "linux";

        let truecolor_env = colorterm == "truecolor" || colorterm == "24bit";
        let color = if set("NO_COLOR") || dumb {
            ColorDepth::None
        } else if tmux {
            // Inside tmux the outer terminal's signals (KITTY_WINDOW_ID, ...) are
            // inherited but prove nothing about what tmux forwards.
            if truecolor_env || probe.tmux_rgb() == Some(true) {
                ColorDepth::TrueColor
            } else if term.contains("256color")
                || probe.terminfo(&term).and_then(|t| t.colors).unwrap_or(0) >= 256
            {
                ColorDepth::Ansi256
            } else {
                // tmux itself always draws 256 colors; screen/tmux TERMs without
                // the suffix are a conservative configuration, honour it.
                ColorDepth::Ansi16
            }
        } else if truecolor_env
            || term.ends_with("-direct")
            || kitty
            || ghostty
            || foot
            || alacritty
            || wezterm
            || iterm
            || vscode
        {
            ColorDepth::TrueColor
        } else if term.contains("256color") {
            ColorDepth::Ansi256
        } else if console || term.is_empty() {
            ColorDepth::Ansi16
        } else {
            match probe.terminfo(&term) {
                Some(Terminfo { rgb: true, .. }) => ColorDepth::TrueColor,
                Some(Terminfo {
                    colors: Some(n), ..
                }) if n >= 256 => ColorDepth::Ansi256,
                _ => ColorDepth::Ansi16,
            }
        };

        // LC_ALL > LC_CTYPE > LANG; the first non-empty one decides. With no
        // locale at all (common under systemd/ssh), assume UTF-8: every terminal
        // cx targets is UTF-8, and an explicit `C`/`POSIX` still selects ASCII.
        let locale_utf8 = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|k| nonempty(k))
            .map(|v| {
                let v = v.to_ascii_lowercase();
                v.contains("utf-8") || v.contains("utf8")
            })
            .unwrap_or(true);
        let unicode = !set("CX_ASCII") && !dumb && !console && locale_utf8;

        let background = match get("CX_THEME").as_deref().map(str::to_ascii_lowercase) {
            Some(v) if v == "light" => Background::Light,
            Some(v) if v == "dark" => Background::Dark,
            _ => get("COLORFGBG")
                .and_then(|v| colorfgbg_background(&v))
                .unwrap_or(Background::Dark),
        };

        let vte_links = get("VTE_VERSION")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .is_some_and(|v| v >= 5000);
        let hyperlinks =
            !tmux && (kitty || foot || ghostty || wezterm || iterm || vte_links || alacritty);
        Self {
            color,
            unicode,
            background,
            tmux,
            termux,
            phone: termux || blink,
            hyperlinks,
            nerd_font_likely: kitty || ghostty || wezterm,
            icons: get("CX_ICONS").as_deref() == Some("1"),
        }
    }

    /// Phone layout: always below 60 columns, and up to 70 on a phone terminal
    /// in portrait (rows > cols x 0.5).
    pub(crate) fn prefers_xs(&self, cols: u16, rows: u16) -> bool {
        cols < 60 || (self.phone && cols < 70 && u32::from(rows) * 2 > u32::from(cols))
    }

    /// One bounded line for debug logs: no environment values, only verdicts.
    #[allow(dead_code)]
    pub(crate) fn summary(&self) -> String {
        format!(
            "caps color={:?} unicode={} bg={:?} tmux={} phone={} links={} nerd={}",
            self.color,
            self.unicode,
            self.background,
            self.tmux,
            self.phone,
            self.hyperlinks,
            self.nerd_font_likely
        )
    }
}

/// `COLORFGBG` is `fg;bg` or `fg;default;bg`. ANSI 0-6 and 8 are dark backgrounds.
fn colorfgbg_background(value: &str) -> Option<Background> {
    let bg = value.rsplit(';').next()?.trim().parse::<u8>().ok()?;
    Some(if bg == 7 || (9..=15).contains(&bg) {
        Background::Light
    } else {
        Background::Dark
    })
}

/// Bounded, read-only probes of the real system.
struct SystemProbe;

impl Probe for SystemProbe {
    fn terminfo(&self, term: &str) -> Option<Terminfo> {
        if term.is_empty()
            || !term
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._+".contains(&b))
        {
            return None;
        }
        let out = bounded_output(
            Command::new("infocmp").args(["-1", "-x", term]),
            Duration::from_millis(100),
        )?;
        let mut info = Terminfo::default();
        for cap in out.lines().map(|l| l.trim().trim_end_matches(',')) {
            if let Some(n) = cap.strip_prefix("colors#") {
                info.colors = if let Some(hex) = n.strip_prefix("0x") {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    n.parse().ok()
                };
            } else if cap == "Tc" || cap == "RGB" || cap.starts_with("RGB#") {
                info.rgb = true;
            }
        }
        Some(info)
    }
    fn tmux_rgb(&self) -> Option<bool> {
        // Read-only query of the user's own tmux client (tmux >= 3.2 knows the
        // format; older versions print nothing and we fall back to 256 colors).
        let out = bounded_output(
            Command::new("tmux").args(["display-message", "-p", "#{client_termfeatures}"]),
            Duration::from_millis(200),
        )?;
        let features = out.trim();
        (!features.is_empty()).then(|| features.split(',').any(|f| f == "RGB"))
    }
}

/// Run a command with null stdin/stderr; give up (and kill it) after `limit`.
/// Output is capped at 64 KiB.
fn bounded_output(command: &mut Command, limit: Duration) -> Option<String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if started.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    let mut out = String::new();
    child
        .stdout
        .take()?
        .take(64 * 1024)
        .read_to_string(&mut out)
        .ok()?;
    Some(out)
}

static CAPS: OnceLock<Caps> = OnceLock::new();

/// The process-wide capabilities, detected once on first use.
#[cfg(not(test))]
pub(crate) fn caps() -> Caps {
    *CAPS.get_or_init(|| Caps::detect(&|k| std::env::var(k).ok(), &SystemProbe))
}

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<Caps>> = const { std::cell::Cell::new(None) };
}

/// Tests: a fixed truecolor/dark/Unicode profile that still honours `NO_COLOR`
/// and `CX_ASCII`, so results never depend on the developer's terminal.
#[cfg(test)]
pub(crate) fn caps() -> Caps {
    OVERRIDE.with(|o| o.get()).unwrap_or_else(|| {
        *CAPS.get_or_init(|| {
            let _ = &SystemProbe; // keep the live probe compiled in tests
            Caps {
                color: if std::env::var_os("NO_COLOR").is_some() {
                    ColorDepth::None
                } else {
                    ColorDepth::TrueColor
                },
                unicode: std::env::var_os("CX_ASCII").is_none(),
                ..Caps::default()
            }
        })
    })
}

/// Run `f` with `caps` in effect on this thread (tests only).
#[cfg(test)]
pub(crate) fn with_caps<R>(caps: Caps, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Caps>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.with(|o| o.set(self.0));
        }
    }
    let _restore = Restore(OVERRIDE.with(|o| o.replace(Some(caps))));
    f()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fake {
        terminfo: Option<Terminfo>,
        tmux_rgb: Option<bool>,
    }
    impl Probe for Fake {
        fn terminfo(&self, _term: &str) -> Option<Terminfo> {
            self.terminfo
        }
        fn tmux_rgb(&self) -> Option<bool> {
            self.tmux_rgb
        }
    }
    fn detect_with(pairs: &[(&str, &str)], probe: &dyn Probe) -> Caps {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Caps::detect(&|k| env.get(k).cloned(), probe)
    }
    fn detect(pairs: &[(&str, &str)]) -> Caps {
        detect_with(pairs, &NoProbe)
    }
    const UTF8: (&str, &str) = ("LANG", "en_US.UTF-8");

    #[test]
    fn color_depth_matrix() {
        use ColorDepth::*;
        let cases: &[(&str, &[(&str, &str)], ColorDepth)] = &[
            ("foot", &[("TERM", "foot"), UTF8], TrueColor),
            ("foot-extra", &[("TERM", "foot-extra"), UTF8], TrueColor),
            (
                "kitty",
                &[("TERM", "xterm-kitty"), ("KITTY_WINDOW_ID", "1"), UTF8],
                TrueColor,
            ),
            (
                "ghostty",
                &[("TERM", "xterm-ghostty"), ("TERM_PROGRAM", "ghostty"), UTF8],
                TrueColor,
            ),
            (
                "alacritty",
                &[("TERM", "alacritty"), ("ALACRITTY_SOCKET", "/x"), UTF8],
                TrueColor,
            ),
            (
                "alacritty-xterm",
                &[
                    ("TERM", "xterm-256color"),
                    ("ALACRITTY_WINDOW_ID", "9"),
                    UTF8,
                ],
                TrueColor,
            ),
            (
                "wezterm",
                &[
                    ("TERM", "xterm-256color"),
                    ("TERM_PROGRAM", "WezTerm"),
                    UTF8,
                ],
                TrueColor,
            ),
            (
                "iterm",
                &[
                    ("TERM", "xterm-256color"),
                    ("TERM_PROGRAM", "iTerm.app"),
                    UTF8,
                ],
                TrueColor,
            ),
            (
                "vscode",
                &[("TERM", "xterm-256color"), ("TERM_PROGRAM", "vscode"), UTF8],
                TrueColor,
            ),
            (
                "colorterm",
                &[("TERM", "xterm-256color"), ("COLORTERM", "truecolor"), UTF8],
                TrueColor,
            ),
            (
                "colorterm-24bit",
                &[("TERM", "xterm"), ("COLORTERM", "24bit"), UTF8],
                TrueColor,
            ),
            ("direct", &[("TERM", "xterm-direct"), UTF8], TrueColor),
            ("xterm-256", &[("TERM", "xterm-256color"), UTF8], Ansi256),
            ("xterm", &[("TERM", "xterm"), UTF8], Ansi16),
            ("unset-term", &[UTF8], Ansi16),
            ("console", &[("TERM", "linux")], Ansi16),
            ("dumb", &[("TERM", "dumb")], None),
            (
                "no-color",
                &[
                    ("TERM", "xterm-kitty"),
                    ("COLORTERM", "truecolor"),
                    ("NO_COLOR", "1"),
                ],
                None,
            ),
            (
                "no-color-empty",
                &[("TERM", "xterm-256color"), ("NO_COLOR", "")],
                None,
            ),
            // tmux inside kitty: inherited kitty markers are not evidence.
            (
                "tmux-in-kitty",
                &[
                    ("TERM", "tmux-256color"),
                    ("TMUX", "/tmp/s,1,0"),
                    ("KITTY_WINDOW_ID", "1"),
                    UTF8,
                ],
                Ansi256,
            ),
            (
                "tmux-colorterm",
                &[
                    ("TERM", "tmux-256color"),
                    ("TMUX", "/tmp/s,1,0"),
                    ("COLORTERM", "truecolor"),
                    UTF8,
                ],
                TrueColor,
            ),
            (
                "tmux-screen",
                &[("TERM", "screen"), ("TMUX", "/tmp/s,1,0"), UTF8],
                Ansi16,
            ),
            (
                "termux",
                &[
                    ("TERM", "xterm-256color"),
                    ("TERMUX_VERSION", "0.118"),
                    ("COLORTERM", "truecolor"),
                    UTF8,
                ],
                TrueColor,
            ),
            (
                "blink",
                &[("TERM", "xterm-256color"), ("TERM_PROGRAM", "Blink"), UTF8],
                Ansi256,
            ),
        ];
        for (name, env, want) in cases {
            assert_eq!(detect(env).color, *want, "{name}");
        }
    }

    #[test]
    fn probes_decide_only_when_the_environment_cannot() {
        let rgb = Fake {
            terminfo: Some(Terminfo {
                colors: Some(256),
                rgb: true,
            }),
            tmux_rgb: Some(true),
        };
        let tmux = [("TERM", "tmux-256color"), ("TMUX", "/tmp/s,1,0"), UTF8];
        assert_eq!(detect_with(&tmux, &rgb).color, ColorDepth::TrueColor);
        let no_rgb = Fake {
            terminfo: None,
            tmux_rgb: Some(false),
        };
        assert_eq!(detect_with(&tmux, &no_rgb).color, ColorDepth::Ansi256);
        // Unknown TERM name: terminfo decides.
        let odd = [("TERM", "rxvt-unicode"), UTF8];
        assert_eq!(detect_with(&odd, &rgb).color, ColorDepth::TrueColor);
        let colors256 = Fake {
            terminfo: Some(Terminfo {
                colors: Some(256),
                rgb: false,
            }),
            tmux_rgb: None,
        };
        assert_eq!(detect_with(&odd, &colors256).color, ColorDepth::Ansi256);
        assert_eq!(detect_with(&odd, &NoProbe).color, ColorDepth::Ansi16);
        // A panicking probe proves the env-decided path never calls it.
        struct Panics;
        impl Probe for Panics {
            fn terminfo(&self, _: &str) -> Option<Terminfo> {
                panic!("terminfo probed")
            }
            fn tmux_rgb(&self) -> Option<bool> {
                panic!("tmux probed")
            }
        }
        for env in [
            &[("TERM", "xterm-kitty"), UTF8][..],
            &[("TERM", "xterm-256color")][..],
            &[("TERM", "dumb")][..],
            &[
                ("TERM", "tmux-256color"),
                ("TMUX", "x"),
                ("COLORTERM", "truecolor"),
            ][..],
        ] {
            detect_with(env, &Panics);
        }
    }

    #[test]
    fn glyph_set_matrix() {
        let cases: &[(&str, &[(&str, &str)], bool)] = &[
            ("utf8", &[("TERM", "foot"), UTF8], true),
            ("utf8-lower", &[("TERM", "foot"), ("LANG", "C.utf8")], true),
            (
                "lang-c",
                &[("TERM", "xterm-256color"), ("LANG", "C")],
                false,
            ),
            (
                "posix",
                &[("TERM", "xterm-256color"), ("LC_ALL", "POSIX"), UTF8],
                false,
            ),
            (
                "lc_all-wins",
                &[("TERM", "xterm"), ("LC_ALL", "en_US.UTF-8"), ("LANG", "C")],
                true,
            ),
            (
                "lc_ctype",
                &[("TERM", "xterm"), ("LC_CTYPE", "en_US.ISO-8859-1"), UTF8],
                false,
            ),
            (
                "empty-lc_all-skipped",
                &[("TERM", "xterm"), ("LC_ALL", ""), UTF8],
                true,
            ),
            ("no-locale", &[("TERM", "xterm-256color")], true),
            (
                "cx-ascii",
                &[("TERM", "xterm-kitty"), ("CX_ASCII", "1"), UTF8],
                false,
            ),
            ("dumb", &[("TERM", "dumb"), UTF8], false),
            ("console", &[("TERM", "linux"), UTF8], false),
        ];
        for (name, env, want) in cases {
            assert_eq!(detect(env).unicode, *want, "{name}");
        }
    }

    #[test]
    fn phone_background_links_and_icons() {
        let termux = detect(&[
            ("TERM", "xterm-256color"),
            ("PREFIX", "/data/data/com.termux/files/usr"),
            UTF8,
        ]);
        assert!(termux.termux && termux.phone);
        assert!(
            termux.prefers_xs(66, 40) && !termux.prefers_xs(66, 20) && !termux.prefers_xs(80, 60)
        );
        let blink = detect(&[("TERM", "xterm-256color"), ("TERM_PROGRAM", "Blink"), UTF8]);
        assert!(blink.phone && !blink.termux);
        let desk = detect(&[("TERM", "foot"), UTF8]);
        assert!(!desk.phone && desk.prefers_xs(40, 20) && !desk.prefers_xs(66, 40));

        assert_eq!(
            detect(&[("CX_THEME", "light")]).background,
            Background::Light
        );
        assert_eq!(
            detect(&[("CX_THEME", "DARK"), ("COLORFGBG", "0;15")]).background,
            Background::Dark
        );
        assert_eq!(
            detect(&[("COLORFGBG", "0;15")]).background,
            Background::Light
        );
        assert_eq!(
            detect(&[("COLORFGBG", "0;default;7")]).background,
            Background::Light
        );
        assert_eq!(
            detect(&[("COLORFGBG", "15;0")]).background,
            Background::Dark
        );
        assert_eq!(
            detect(&[("COLORFGBG", "junk")]).background,
            Background::Dark
        );
        assert_eq!(detect(&[]).background, Background::Dark);

        assert!(detect(&[("TERM", "foot")]).hyperlinks);
        assert!(detect(&[("TERM", "xterm-256color"), ("VTE_VERSION", "7600")]).hyperlinks);
        assert!(!detect(&[("TERM", "xterm-256color"), ("VTE_VERSION", "4000")]).hyperlinks);
        assert!(
            !detect(&[
                ("TERM", "tmux-256color"),
                ("TMUX", "x"),
                ("KITTY_WINDOW_ID", "1")
            ])
            .hyperlinks
        );
        assert!(!detect(&[("TERM", "xterm-256color")]).hyperlinks);

        let kitty = detect(&[("TERM", "xterm-kitty")]);
        assert!(kitty.nerd_font_likely && !kitty.icons);
        assert!(detect(&[("CX_ICONS", "1")]).icons);
        assert!(!detect(&[("CX_ICONS", "yes")]).icons);
        // The summary is one bounded line without environment values.
        let s = detect(&[("TERM", "xterm-kitty"), ("HOME", "/home/secret")]).summary();
        assert!(!s.contains('\n') && !s.contains("secret") && s.len() < 160);
    }

    #[test]
    fn override_is_scoped_and_restored() {
        let before = caps();
        let ascii = Caps {
            unicode: false,
            color: ColorDepth::None,
            ..Caps::default()
        };
        with_caps(ascii, || {
            assert_eq!(caps(), ascii);
            with_caps(Caps::default(), || assert_eq!(caps(), Caps::default()));
            assert_eq!(caps(), ascii);
        });
        assert_eq!(caps(), before);
    }
}
