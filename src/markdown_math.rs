//! Small, inert TeX math preview. Unsupported expressions retain their source in the caller.
//!
//! This deliberately supports a readable Unicode subset, not TeX layout or execution.
const MAX_INPUT: usize = 4096;
const MAX_OUTPUT: usize = 8192;
const MAX_DEPTH: usize = 24;

/// Render a supported TeX expression as terminal text, or reject the entire expression.
/// The caller must keep the original source when this returns `None`.
pub fn render_math(source: &str) -> Option<String> {
    if source.len() > MAX_INPUT
        || source
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return None;
    }
    let mut parser = Parser {
        chars: source.chars().collect(),
        at: 0,
    };
    let rendered = parser.sequence(false, 0)?;
    (parser.at == parser.chars.len() && rendered.len() <= MAX_OUTPUT).then_some(rendered)
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn sequence(&mut self, grouped: bool, depth: usize) -> Option<String> {
        if depth > MAX_DEPTH {
            return None;
        }
        let mut out = String::new();
        let mut has_base = false;
        let mut superscript_seen = false;
        let mut subscript_seen = false;
        while let Some(&ch) = self.chars.get(self.at) {
            if ch == '}' {
                if !grouped {
                    return None;
                }
                self.at += 1;
                return Some(out);
            }
            if ch == '^' || ch == '_' {
                if !has_base || (ch == '^' && superscript_seen) || (ch == '_' && subscript_seen) {
                    return None;
                }
                superscript_seen |= ch == '^';
                subscript_seen |= ch == '_';
                self.at += 1;
                let value = self.argument(depth + 1)?;
                if value.is_empty() {
                    return None;
                }
                out.push_str(&script(&value, ch == '^'));
            } else {
                out.push_str(&self.atom(depth)?);
                if !ch.is_whitespace() {
                    has_base = true;
                    superscript_seen = false;
                    subscript_seen = false;
                }
            }
            if out.len() > MAX_OUTPUT {
                return None;
            }
        }
        (!grouped).then_some(out)
    }

    fn skip_space(&mut self) {
        while self.chars.get(self.at).is_some_and(|c| c.is_whitespace()) {
            self.at += 1;
        }
    }

    fn argument(&mut self, depth: usize) -> Option<String> {
        self.skip_space();
        if matches!(self.chars.get(self.at), None | Some('}' | '^' | '_')) {
            return None;
        }
        self.atom(depth)
    }

    fn required_group(&mut self, depth: usize) -> Option<String> {
        self.skip_space();
        if self.chars.get(self.at) != Some(&'{') {
            return None;
        }
        self.atom(depth)
    }

    fn atom(&mut self, depth: usize) -> Option<String> {
        if depth > MAX_DEPTH {
            return None;
        }
        let ch = *self.chars.get(self.at)?;
        self.at += 1;
        match ch {
            '{' => self.sequence(true, depth + 1),
            '}' | '$' | '^' | '_' | '&' | '%' | '#' => None,
            '\\' => self.command(depth + 1),
            '\n' | '\r' | '\t' => Some(" ".into()),
            _ => Some(ch.to_string()),
        }
    }

    fn command(&mut self, depth: usize) -> Option<String> {
        let start = self.at;
        while self
            .chars
            .get(self.at)
            .is_some_and(char::is_ascii_alphabetic)
        {
            self.at += 1;
        }
        if self.at == start {
            let escaped = *self.chars.get(self.at)?;
            self.at += 1;
            return match escaped {
                '{' | '}' | '_' | '%' | '#' | '$' | '&' | '|' => Some(escaped.to_string()),
                ',' | ':' | ';' | ' ' => Some(" ".into()),
                '!' => Some(String::new()),
                _ => None,
            };
        }
        let name: String = self.chars[start..self.at].iter().collect();
        let text = match name.as_str() {
            "frac" | "dfrac" | "tfrac" => {
                let top = self.required_group(depth)?;
                let bottom = self.required_group(depth)?;
                if top.is_empty() || bottom.is_empty() {
                    return None;
                }
                return Some(format!("({top})/({bottom})"));
            }
            "sqrt" => {
                let value = self.required_group(depth)?;
                if value.is_empty() {
                    return None;
                }
                return Some(format!("√({value})"));
            }
            "mathrm" | "mathit" | "mathbf" | "text" | "operatorname" => {
                return self.required_group(depth);
            }
            "alpha" => "α",
            "beta" => "β",
            "gamma" => "γ",
            "delta" => "δ",
            "epsilon" => "ϵ",
            "varepsilon" => "ε",
            "zeta" => "ζ",
            "eta" => "η",
            "theta" => "θ",
            "vartheta" => "ϑ",
            "iota" => "ι",
            "kappa" => "κ",
            "lambda" => "λ",
            "mu" => "μ",
            "nu" => "ν",
            "xi" => "ξ",
            "pi" => "π",
            "varpi" => "ϖ",
            "rho" => "ρ",
            "varrho" => "ϱ",
            "sigma" => "σ",
            "varsigma" => "ς",
            "tau" => "τ",
            "upsilon" => "υ",
            "phi" => "ϕ",
            "varphi" => "φ",
            "chi" => "χ",
            "psi" => "ψ",
            "omega" => "ω",
            "Gamma" => "Γ",
            "Delta" => "Δ",
            "Theta" => "Θ",
            "Lambda" => "Λ",
            "Xi" => "Ξ",
            "Pi" => "Π",
            "Sigma" => "Σ",
            "Upsilon" => "Υ",
            "Phi" => "Φ",
            "Psi" => "Ψ",
            "Omega" => "Ω",
            "times" => "×",
            "cdot" => "·",
            "div" => "÷",
            "pm" => "±",
            "mp" => "∓",
            "le" | "leq" => "≤",
            "ge" | "geq" => "≥",
            "ne" | "neq" => "≠",
            "approx" => "≈",
            "equiv" => "≡",
            "propto" => "∝",
            "sim" => "∼",
            "infty" => "∞",
            "partial" => "∂",
            "nabla" => "∇",
            "sum" => "∑",
            "prod" => "∏",
            "int" => "∫",
            "oint" => "∮",
            "in" => "∈",
            "notin" => "∉",
            "subset" => "⊂",
            "subseteq" => "⊆",
            "supset" => "⊃",
            "supseteq" => "⊇",
            "cup" => "∪",
            "cap" => "∩",
            "emptyset" => "∅",
            "forall" => "∀",
            "exists" => "∃",
            "neg" => "¬",
            "land" => "∧",
            "lor" => "∨",
            "to" | "rightarrow" => "→",
            "leftarrow" => "←",
            "leftrightarrow" => "↔",
            "Rightarrow" | "implies" => "⇒",
            "Leftarrow" => "⇐",
            "Leftrightarrow" | "iff" => "⇔",
            "ldots" | "dots" => "…",
            "cdots" => "⋯",
            "langle" => "⟨",
            "rangle" => "⟩",
            "lvert" | "rvert" | "vert" => "|",
            "lVert" | "rVert" | "Vert" => "‖",
            "sin" => "sin",
            "cos" => "cos",
            "tan" => "tan",
            "log" => "log",
            "ln" => "ln",
            "exp" => "exp",
            "lim" => "lim",
            "max" => "max",
            "min" => "min",
            "quad" => "  ",
            "qquad" => "    ",
            // Unsupported sizing, matrices, macros and package commands remain raw.
            _ => return None,
        };
        // Preserve source spacing to keep terminal identifiers (e.g. sin x) readable.
        Some(text.into())
    }
}

fn script(value: &str, superscript: bool) -> String {
    let from = if superscript {
        "0123456789+-=()in"
    } else {
        "0123456789+-=()aeioruvxβγρφχ"
    };
    let to = if superscript {
        "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁼⁽⁾ⁱⁿ"
    } else {
        "₀₁₂₃₄₅₆₇₈₉₊₋₌₍₎ₐₑᵢₒᵣᵤᵥₓᵦᵧᵨᵩᵪ"
    };
    let mapped: Option<String> = value
        .chars()
        .map(|c| {
            from.chars()
                .position(|v| v == c)
                .and_then(|i| to.chars().nth(i))
        })
        .collect();
    mapped.unwrap_or_else(|| format!("{}({value})", if superscript { '^' } else { '_' }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_formula_and_greek() {
        assert_eq!(render_math(r"E = mc^2"), Some("E = mc²".into()));
        assert_eq!(
            render_math(r"\alpha_1 + \beta^{12} \leq \infty"),
            Some("α₁ + β¹² ≤ ∞".into())
        );
        assert_eq!(render_math(r"\sum_{i=0}^{n} x_i"), Some("∑ᵢ₌₀ⁿ xᵢ".into()));
    }

    #[test]
    fn fractions_roots_and_scripts_preserve_grouping() {
        assert_eq!(
            render_math(r"\frac{a+b}{\sqrt{x^2}}"),
            Some("(a+b)/(√(x²))".into())
        );
        assert_eq!(
            render_math(r"x_{velocity}^{ab}"),
            Some("x_(velocity)^(ab)".into())
        );
        assert_eq!(
            render_math(r"\text{speed} = \frac{d}{t}"),
            Some("speed = (d)/(t)".into())
        );
    }

    #[test]
    fn unsupported_is_whole_expression_fallback() {
        for input in [
            r"x + \unknown{y}",
            r"\begin{matrix}a&b\end{matrix}",
            r"\sqrt[3]{x}",
            r"\hat{x}",
            r"\left(x\right)",
            r"\input{file}",
        ] {
            assert_eq!(render_math(input), None, "{input}");
        }
    }

    #[test]
    fn malformed_nesting_and_arguments() {
        for input in [
            "{x",
            "x}",
            "x^",
            "x_}",
            "\\",
            r"\frac{x}",
            r"\frac{x}{}",
            r"\sqrt{}",
            "x^{}",
            "x^^2",
            "^2",
            "_1",
            "x^2^3",
            "x_1_2",
        ] {
            assert_eq!(render_math(input), None, "{input}");
        }
    }

    #[test]
    fn bounded_and_control_free() {
        assert_eq!(render_math(&"a".repeat(MAX_INPUT + 1)), None);
        assert_eq!(
            render_math(&format!(
                "{}x{}",
                "{".repeat(MAX_DEPTH + 1),
                "}".repeat(MAX_DEPTH + 1)
            )),
            None
        );
        assert_eq!(render_math("x\u{1b}[31m"), None);
        assert_eq!(render_math("x\ny"), Some("x y".into()));
        assert!(render_math(&"x".repeat(MAX_INPUT)).is_some());
    }

    #[test]
    fn escaped_literals_remain_inert() {
        assert_eq!(render_math(r"\{x\} \% \_ \$"), Some("{x} % _ $".into()));
        assert_eq!(
            render_math(r"\operatorname{rank}(A)"),
            Some("rank(A)".into())
        );
    }

    #[test]
    fn deterministic_parser_never_panics_on_short_malformed_inputs() {
        let alphabet = ['{', '}', '^', '_', '\\', 'x', '1', '$'];
        for encoded in 0..8usize.pow(5) {
            let mut value = encoded;
            let input: String = (0..5)
                .map(|_| {
                    let ch = alphabet[value % alphabet.len()];
                    value /= alphabet.len();
                    ch
                })
                .collect();
            if let Some(rendered) = render_math(&input) {
                assert!(rendered.len() <= MAX_OUTPUT);
                assert!(!rendered.chars().any(char::is_control));
            }
        }
    }
}
