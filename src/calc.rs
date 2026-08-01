//! Arithmetic for the calculator panel: parse a typed expression, evaluate it.
//!
//! Split from the panel for the same reason `quote::parse_chart` is split from
//! the HTTP call — the interesting behaviour is in the parsing, and it is worth
//! testing without building a terminal.
//!
//! # Why a parser and not two registers
//!
//! A pocket calculator needs no parser: pressing an operator evaluates whatever
//! came before it, so `2 + 3 * 4` is 20. That is genuinely what a *basic*
//! calculator does, and it was the other candidate. It lost because of who is
//! typing: somebody at a terminal who reaches for this instead of `bc` or
//! `python3 -c` expects `2 + 3 * 4` to be 14, and would read 20 as a bug rather
//! than as a design. Precedence is not the scientific end of the wedge — memory
//! keys, percent and functions are, and none of those are here.
//!
//! Parentheses come along nearly free once there is a `factor` rule, and their
//! absence would be the surprising thing in an expression that already honours
//! precedence.
//!
//! # Bounds
//!
//! Everything here reads text somebody typed, so the sizes are capped rather
//! than trusted. That is the lesson from the untrusted-input pass: the interior
//! arithmetic is usually right, and what is missing is a bound.
//!
//! Nesting is bounded **explicitly** rather than by the stack. `(((((…` is one
//! key held down, and a stack overflow aborts the process — there is no
//! catching it and no error to show. A depth limit turns that into a message.

/// Longest expression that can be typed.
///
/// Far past anything a person writes by hand; it exists so that no later
/// bound has to reason about unbounded length.
pub const MAX_LEN: usize = 256;

/// Deepest nesting of parentheses.
///
/// Not a stack-depth estimate — a flat refusal well below anything that could
/// threaten one. Sixteen is more nesting than arithmetic anybody can read.
const MAX_DEPTH: usize = 16;

/// Significant digits kept when a result is turned into text.
///
/// Twelve, because that is what makes binary floating point behave the way a
/// calculator's user expects: `0.1 + 0.2` is 0.30000000000000004 in an `f64`,
/// and every person who opens a calculator tries it. Rounding the *display*
/// rather than the arithmetic is what desk calculators have always done.
const SIGNIFICANT_DIGITS: usize = 12;

/// Why an expression could not be evaluated.
///
/// One short phrase each, written to be shown where the answer would go. They
/// name what is wrong with the expression rather than where the parser was,
/// because "unexpected token at 7" is a compiler's kind of honesty and no use
/// to somebody who just wants the sum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalcError {
    Empty,
    TooLong,
    TooDeep,
    UnbalancedParens,
    /// An operator with nothing after it — the state every half-typed
    /// expression is in, so it must read as unfinished rather than as broken.
    Incomplete,
    DivideByZero,
    /// A character that means nothing here.
    Unexpected(char),
    /// The arithmetic left the range an `f64` can describe.
    OutOfRange,
}

impl CalcError {
    /// The phrase shown in place of the answer.
    ///
    /// Short on purpose: these are drawn in the calculator's result column,
    /// which is fourteen cells, and a reason that has to be ellipsised is a
    /// reason nobody can read. `cannot divide by zero` came out as
    /// `cannot divid…`, which is worse than the terse version.
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "nothing yet".to_string(),
            Self::TooLong => "too long".to_string(),
            Self::TooDeep => "too nested".to_string(),
            Self::UnbalancedParens => "unclosed (".to_string(),
            Self::Incomplete => "unfinished".to_string(),
            Self::DivideByZero => "\u{00f7} by zero".to_string(),
            Self::Unexpected(c) => format!("{c}?"),
            Self::OutOfRange => "too large".to_string(),
        }
    }
}

/// Evaluate a typed expression.
pub fn evaluate(text: &str) -> Result<f64, CalcError> {
    if text.len() > MAX_LEN {
        return Err(CalcError::TooLong);
    }
    if text.trim().is_empty() {
        return Err(CalcError::Empty);
    }

    let chars: Vec<char> = text.chars().collect();
    let mut parser = Parser {
        chars: &chars,
        at: 0,
        depth: 0,
    };
    let value = parser.expr()?;
    parser.skip_spaces();
    if parser.at < parser.chars.len() {
        // The only way to be left mid-input with no error is a stray closing
        // bracket, which the recursive descent has no rule to consume.
        return Err(match parser.chars[parser.at] {
            ')' => CalcError::UnbalancedParens,
            c => CalcError::Unexpected(c),
        });
    }
    if !value.is_finite() {
        return Err(CalcError::OutOfRange);
    }
    Ok(value)
}

struct Parser<'a> {
    chars: &'a [char],
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn skip_spaces(&mut self) {
        while self.chars.get(self.at) == Some(&' ') {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_spaces();
        self.chars.get(self.at).copied()
    }

    /// `term (('+' | '-') term)*`
    fn expr(&mut self) -> Result<f64, CalcError> {
        let mut value = self.term()?;
        while let Some(op @ ('+' | '-')) = self.peek() {
            self.at += 1;
            let rhs = self.term()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Ok(value)
    }

    /// `factor (('*' | '/') factor)*`
    fn term(&mut self) -> Result<f64, CalcError> {
        let mut value = self.factor()?;
        // `x` and `×` are accepted for multiply because both are what people
        // reach for; `÷` for the same reason. None of them can mean anything
        // else here.
        while let Some(op @ ('*' | '/' | 'x' | '\u{00D7}' | '\u{00F7}')) = self.peek() {
            self.at += 1;
            let rhs = self.factor()?;
            if matches!(op, '*' | 'x' | '\u{00D7}') {
                value *= rhs;
            } else {
                if rhs == 0.0 {
                    return Err(CalcError::DivideByZero);
                }
                value /= rhs;
            }
        }
        Ok(value)
    }

    /// `'-' factor | '+' factor | number | '(' expr ')'`
    fn factor(&mut self) -> Result<f64, CalcError> {
        match self.peek() {
            // Nothing left, or a bracket closing where a value was due. Both
            // are the same thing to the reader — the sum stops before it says
            // anything — so they get the same word rather than a taxonomy.
            None | Some(')') => Err(CalcError::Incomplete),
            Some('-') => {
                self.at += 1;
                Ok(-self.factor()?)
            }
            // A leading `+` is meaningless but harmless, and refusing it would
            // be pedantry aimed at somebody who typed what they meant.
            Some('+') => {
                self.at += 1;
                self.factor()
            }
            Some('(') => {
                self.at += 1;
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(CalcError::TooDeep);
                }
                let value = self.expr()?;
                if self.peek() != Some(')') {
                    return Err(CalcError::UnbalancedParens);
                }
                self.at += 1;
                self.depth -= 1;
                Ok(value)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => self.number(),
            Some(c) => Err(CalcError::Unexpected(c)),
        }
    }

    fn number(&mut self) -> Result<f64, CalcError> {
        let start = self.at;
        let mut seen_dot = false;
        while let Some(&c) = self.chars.get(self.at) {
            if c.is_ascii_digit() {
                self.at += 1;
            } else if c == '.' && !seen_dot {
                seen_dot = true;
                self.at += 1;
            } else {
                break;
            }
        }
        let text: String = self.chars[start..self.at].iter().collect();
        // A bare `.` reaches here, and `parse` refuses it. So does a second dot
        // in the same number, which stops the loop above and is then caught as
        // an unexpected character by the caller.
        text.parse::<f64>().map_err(|_| CalcError::Incomplete)
    }
}

/// A result as text, at the precision a calculator shows.
///
/// Rounded to [`SIGNIFICANT_DIGITS`] and stripped of trailing zeros, so
/// `0.1 + 0.2` reads `0.3` rather than exposing the binary representation.
/// Whole numbers keep no decimal point.
pub fn format_result(value: f64) -> String {
    if value == 0.0 {
        // Covers `-0.0`, which `{}` prints as `-0` — an answer nobody wants to
        // see and which reads as a bug in the arithmetic.
        return "0".to_string();
    }
    if !value.is_finite() {
        return "—".to_string();
    }

    let text = format!("{value:.prec$e}", prec = SIGNIFICANT_DIGITS - 1);
    // Back through `f64` so the rounded value is what gets laid out in decimal;
    // formatting the original again would reintroduce what was just rounded off.
    let rounded: f64 = text.parse().unwrap_or(value);

    let mut out = format!("{rounded}");
    if out.contains('e') {
        // Rust's `{}` only reaches for an exponent at extremes, and where it
        // does there is no decimal form worth showing instead.
        return out;
    }
    if out.contains('.') {
        while out.ends_with('0') {
            out.pop();
        }
        if out.ends_with('.') {
            out.pop();
        }
    }
    out
}

/// A result as text, guaranteed to fit `width` cells.
///
/// **A calculator is the one panel where a clipped number is unambiguously a
/// lie.** Everywhere else in mirador a value cut short reads as a narrow
/// terminal; here `1234567` cut to `12345` is a plausible answer that is not
/// the one that was worked out, and an ellipsis does not help — `1.2345…`
/// hides the magnitude, which is the part that was worth knowing.
///
/// So this never truncates. When the decimal form will not fit it moves to
/// scientific notation, which is short, honest about being approximate, and
/// still says how big the number is. Below the width even that needs, it gives
/// up and says so rather than showing a digit somebody might read as an answer.
pub fn fit_result(value: f64, width: usize) -> String {
    let plain = format_result(value);
    if plain.chars().count() <= width {
        return plain;
    }

    // Drop a significant digit at a time until the exponent form fits. The
    // exponent itself is not negotiable, so this bottoms out rather than
    // looping for ever.
    for digits in (0..=SIGNIFICANT_DIGITS.saturating_sub(1)).rev() {
        let candidate = format!("{value:.digits$e}");
        if candidate.chars().count() <= width {
            return candidate;
        }
    }
    "—".to_string()
}

// Exact comparison is deliberate here: these are answers, not measurements.
// A calculator that is approximately right is broken.
#[allow(clippy::float_cmp)]
#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> f64 {
        evaluate(text).unwrap_or_else(|e| panic!("{text:?} failed: {:?}", e.message()))
    }

    #[test]
    fn precedence_is_the_arithmetic_kind() {
        // The decision the whole module turns on. A pocket calculator gives 20.
        assert_eq!(ok("2 + 3 * 4"), 14.0);
        assert_eq!(ok("2 * 3 + 4"), 10.0);
        assert_eq!(ok("100 - 10 / 2"), 95.0);
    }

    #[test]
    fn brackets_override_precedence() {
        assert_eq!(ok("(2 + 3) * 4"), 20.0);
        assert_eq!(ok("((1 + 2) * (3 + 4))"), 21.0);
    }

    #[test]
    fn the_display_hides_the_binary_representation() {
        // The first thing anybody types into a new calculator.
        assert_eq!(format_result(ok("0.1 + 0.2")), "0.3");
        assert_eq!(format_result(ok("1 / 3")), "0.333333333333");
        assert_eq!(format_result(ok("2 / 2")), "1");
        assert_eq!(format_result(ok("10 * 10")), "100");
    }

    #[test]
    fn a_negative_zero_is_shown_as_zero() {
        assert_eq!(format_result(ok("0 * -1")), "0");
    }

    #[test]
    fn unary_minus_binds_tighter_than_the_operators() {
        assert_eq!(ok("-5 + 3"), -2.0);
        assert_eq!(ok("3 - -5"), 8.0);
        assert_eq!(ok("-(2 + 3)"), -5.0);
    }

    #[test]
    fn the_alternative_operator_glyphs_work() {
        assert_eq!(ok("6 x 7"), 42.0);
        assert_eq!(ok("6 \u{00D7} 7"), 42.0);
        assert_eq!(ok("84 \u{00F7} 2"), 42.0);
    }

    #[test]
    fn dividing_by_zero_is_an_error_rather_than_infinity() {
        assert_eq!(evaluate("1 / 0"), Err(CalcError::DivideByZero));
        assert_eq!(evaluate("1 / (2 - 2)"), Err(CalcError::DivideByZero));
    }

    #[test]
    fn a_half_typed_expression_reads_as_unfinished() {
        // This is the state the panel is in for most of every keystroke, so it
        // has to be the gentlest of the errors rather than the loudest.
        assert_eq!(evaluate("2 +"), Err(CalcError::Incomplete));
        assert_eq!(evaluate("2 * "), Err(CalcError::Incomplete));
        assert_eq!(evaluate("(2 + 3"), Err(CalcError::UnbalancedParens));
    }

    #[test]
    fn a_stray_closing_bracket_is_refused() {
        assert_eq!(evaluate("2 + 3)"), Err(CalcError::UnbalancedParens));
        assert_eq!(evaluate(")"), Err(CalcError::Incomplete));
    }

    #[test]
    fn nonsense_names_the_character() {
        assert_eq!(evaluate("2 & 3"), Err(CalcError::Unexpected('&')));
        assert!(
            CalcError::Unexpected('&').message().contains('&'),
            "the message has to name what was typed"
        );
    }

    /// The bound that is not about correctness but about not aborting.
    ///
    /// Recursive descent recurses once per bracket, and a stack overflow is an
    /// abort — no unwinding, no error, no message. `(((((…` is one key held
    /// down. Checked at a depth far past the limit so this fails by refusing
    /// rather than by dying.
    #[test]
    fn deep_nesting_is_refused_rather_than_overflowing_the_stack() {
        let deep = format!("{}1{}", "(".repeat(120), ")".repeat(120));
        assert_eq!(evaluate(&deep), Err(CalcError::TooDeep));

        // And the limit is generous enough that nothing real reaches it.
        let ordinary = format!("{}1{}", "(".repeat(8), ")".repeat(8));
        assert_eq!(ok(&ordinary), 1.0);
    }

    #[test]
    fn an_over_long_expression_is_refused_by_length() {
        let long = "1+".repeat(MAX_LEN);
        assert_eq!(evaluate(&long), Err(CalcError::TooLong));
    }

    /// `OutOfRange` cannot be reached by typing, and the guard stays anyway.
    ///
    /// Worth recording rather than leaving to be rediscovered: within
    /// [`MAX_LEN`] there is no expression that overflows an `f64`. The largest
    /// magnitude 256 characters can express is about 1e252 — 126 two-digit
    /// numbers multiplied together — against an `f64`'s ceiling near 1e308, and
    /// `e` notation is not part of the input language so no literal shortcut
    /// exists. Division cannot get there either: the smallest divisor that fits
    /// is around 1e-250.
    ///
    /// The check in `evaluate` is therefore latent, and kept for the same
    /// reason as the capacity-zero guard in `samples`: it costs a line, and the
    /// bound it depends on is a constant somebody may raise. What *is* testable
    /// is that the formatting never prints `inf` if it ever does arrive.
    #[test]
    fn a_non_finite_result_is_never_printed_as_a_number() {
        assert_eq!(format_result(f64::INFINITY), "\u{2014}");
        assert_eq!(format_result(f64::NEG_INFINITY), "\u{2014}");
        assert_eq!(format_result(f64::NAN), "\u{2014}");

        // And the length bound is what makes the above unreachable in practice,
        // so it is pinned here rather than only in prose.
        let widest = "99 * ".repeat(50) + "99";
        assert!(widest.len() <= MAX_LEN);
        assert!(
            evaluate(&widest).is_ok_and(f64::is_finite),
            "the longest product that fits must still be a finite number"
        );
    }

    /// The rule this panel exists under: never show a number that is not the
    /// answer.
    #[test]
    fn a_result_too_wide_goes_scientific_rather_than_being_cut() {
        let value = 123_456_789_012.0;
        let plain = format_result(value);
        assert!(
            plain.chars().count() > 8,
            "the fixture must not already fit"
        );

        let fitted = fit_result(value, 8);
        assert!(
            fitted.chars().count() <= 8,
            "{fitted:?} does not fit in 8 cells"
        );
        assert!(
            fitted.contains('e'),
            "a number that will not fit must become scientific, not truncated; got {fitted:?}"
        );
        assert!(
            !fitted.starts_with("1234"),
            "{fitted:?} is a prefix of the real answer, which reads as a different number"
        );
    }

    #[test]
    fn a_result_that_fits_is_left_alone() {
        assert_eq!(fit_result(42.0, 20), "42");
        assert_eq!(fit_result(0.3, 20), "0.3");
    }

    /// Below the width even an exponent needs, there is no honest digit to show.
    #[test]
    fn an_impossible_width_gives_up_rather_than_lying() {
        let out = fit_result(123_456_789_012.0, 2);
        assert!(
            !out.chars().next().is_some_and(|c| c.is_ascii_digit()),
            "{out:?} starts with a digit, which reads as an answer"
        );
    }

    /// Generated input, in the shape the untrusted-input pass established:
    /// fragments chosen because of what the parser does with them, not sampled
    /// from all possible bytes.
    #[test]
    fn generated_expressions_never_panic_or_hang() {
        const PIECES: &[&str] = &[
            "1",
            "0",
            ".",
            "..",
            "+",
            "-",
            "*",
            "/",
            "(",
            ")",
            " ",
            "x",
            "\u{00D7}",
            "\u{00F7}",
            "99999999",
            "0.0000001",
            "&",
            "\u{65E5}",
            "\u{1F31E}",
            "\u{0301}",
            "e",
            "--",
            "()",
        ];
        let mut seed = 12_345u64;
        for _ in 0..20_000 {
            let mut text = String::new();
            // A deterministic shuffle: the corpus is the point, not the source
            // of randomness, and a fixed seed keeps a failure reproducible.
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let count = (seed >> 33) as usize % 12;
            for i in 0..count {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                text.push_str(PIECES[(seed >> 33) as usize % PIECES.len()]);
                let _ = i;
            }
            // The contract is only that it returns. Whether it returns a value
            // or an error is the parser's business.
            if let Ok(value) = evaluate(&text) {
                assert!(
                    value.is_finite(),
                    "{text:?} evaluated to {value}, which should have been an error"
                );
                // Formatting must survive whatever evaluation produced.
                let _ = format_result(value);
                for width in [0usize, 1, 3, 8, 40] {
                    let fitted = fit_result(value, width);
                    assert!(
                        fitted.chars().count() <= width.max(1),
                        "{text:?} at width {width} produced {fitted:?}"
                    );
                }
            }
        }
    }
}
