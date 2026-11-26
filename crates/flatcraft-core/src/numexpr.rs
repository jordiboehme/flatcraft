//! A small calculator for number fields: numbers, `+ - * /`, brackets,
//! unit suffixes and relative edits (`+=10`). Hand-written, so it never
//! panics and needs no dependency. Results are in the field's own unit
//! (millimetres for lengths, degrees for angles).

use std::fmt;

/// What a field holds, which decides the units it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A plain number: no unit.
    Plain,
    /// Millimetres: `mm`, `cm`, `in` (or `"`).
    Length,
    /// Degrees: `deg` (or `°`).
    Angle,
    /// A percentage: `%`.
    Percent,
}

/// A number field: its kind, its range and whether it holds whole numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub kind: Kind,
    /// The smallest value it takes.
    pub min: f64,
    /// The largest value it takes.
    pub max: f64,
    /// The value must be more than 0 (a size, a scale).
    pub positive: bool,
    /// It holds whole numbers: the result is rounded to the nearest one.
    pub integer: bool,
    /// An angle of a turn: reduced into [0, 360) after the check against
    /// [`MAX_TURN_DEG`].
    pub turn: bool,
}

/// The largest angle a turn field takes either way; larger numbers are not
/// exact.
pub const MAX_TURN_DEG: f64 = 1e9;

/// The longest text the calculator reads, in characters.
pub const MAX_CHARS: usize = 200;

/// The deepest nesting of brackets and signs it reads.
const MAX_DEPTH: usize = 64;

impl Field {
    /// A field of `kind` that takes any finite value.
    pub const fn new(kind: Kind) -> Self {
        Field {
            kind,
            min: f64::NEG_INFINITY,
            max: f64::INFINITY,
            positive: false,
            integer: false,
            turn: false,
        }
    }

    /// A length in millimetres.
    pub const fn length() -> Self {
        Self::new(Kind::Length)
    }

    /// A plain number.
    pub const fn plain() -> Self {
        Self::new(Kind::Plain)
    }

    /// Only values from `min` to `max`.
    pub const fn range(mut self, min: f64, max: f64) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// Only values above 0.
    pub const fn positive(mut self) -> Self {
        self.positive = true;
        self
    }

    /// Whole numbers.
    pub const fn integer(mut self) -> Self {
        self.integer = true;
        self
    }

    /// The angle of a turn, reduced into [0, 360).
    pub const fn turn() -> Self {
        let mut f = Self::new(Kind::Angle);
        f.turn = true;
        f
    }

    /// Reads `text` and checks the result against the field: the value to
    /// apply, in the field's unit. `current` is the field's value, used by
    /// a relative edit (`+=10`); `None` when there is no single value (a
    /// mixed selection).
    pub fn eval(&self, text: &str, current: Option<f64>) -> Result<f64, Error> {
        let v = eval(text, self.kind, current)?;
        self.check(text, v)
    }

    fn check(&self, text: &str, v: f64) -> Result<f64, Error> {
        let mut v = v;
        let unit = match self.kind {
            Kind::Length => " mm",
            Kind::Angle => " deg",
            Kind::Percent => " %",
            Kind::Plain => "",
        };
        if self.turn {
            if v.abs() > MAX_TURN_DEG {
                return Err(Error::range(format!(
                    "The angle {} is too large. Type at most 1000000000 degrees either way; full turns of 360 count as 0, so 370 is 10.",
                    short(text.trim())
                )));
            }
            v = v.rem_euclid(360.0);
            // rem_euclid of a tiny negative number can give 360 itself.
            if v >= 360.0 {
                v = 0.0;
            }
        }
        if self.integer {
            v = v.round();
        }
        if self.positive && v <= 0.0 {
            return Err(Error::range(format!(
                "{} is too small: give a value above 0{unit}.",
                show(v)
            )));
        }
        if v < self.min {
            return Err(Error::range(format!(
                "{} is too small: give at least {}{unit}.",
                show(v),
                show(self.min)
            )));
        }
        if v > self.max {
            return Err(Error::range(format!(
                "{} is too large: give at most {}{unit}.",
                show(v),
                show(self.max)
            )));
        }
        // No negative zero.
        Ok(if v == 0.0 { 0.0 } else { v })
    }
}

/// Why a text is not a value. Its text is a short sentence for the person.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    /// Where in the text the problem is, in characters from the start;
    /// `None` when it is about the result, not the text.
    pub pos: Option<usize>,
    pub message: String,
}

impl Error {
    fn range(message: String) -> Self {
        Error { pos: None, message }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// A number for a message: at most 4 decimals, no trailing zeros.
fn show(v: f64) -> String {
    if v.abs() >= 1e15 {
        return format!("{v:e}");
    }
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// The text for a message, cut after 20 characters.
fn short(text: &str) -> String {
    let n = text.chars().count();
    if n <= 24 {
        text.to_string()
    } else {
        let head: String = text.chars().take(20).collect();
        format!("{head}...")
    }
}

/// Reads `text` as a calculation in a field of `kind`. A leading `+=`,
/// `-=`, `*=` or `/=` applies the rest to `current`. The result is in the
/// field's unit; it is always finite.
pub fn eval(text: &str, kind: Kind, current: Option<f64>) -> Result<f64, Error> {
    let shown = short(text.trim());
    let syntax = |pos: usize, what: String| Error {
        pos: Some(pos),
        message: format!("Can't read '{shown}': {what}."),
    };
    // Checked before the text is copied, so a huge paste costs nothing.
    if text.chars().nth(MAX_CHARS).is_some() {
        return Err(syntax(
            MAX_CHARS,
            format!("the text is too long (at most {MAX_CHARS} characters)"),
        ));
    }
    let typed: Vec<char> = text.chars().collect();
    let chars = typed.clone();
    if chars.iter().all(|c| c.is_whitespace()) {
        return Err(Error {
            pos: Some(0),
            message: "The field is empty: type a number.".into(),
        });
    }
    // A comma is the decimal point when there is no dot.
    let has_dot = chars.contains(&'.');
    let mut chars = chars;
    for (i, c) in chars.iter_mut().enumerate() {
        if *c == ',' {
            if has_dot {
                return Err(syntax(
                    i,
                    "use a dot for decimals, not a comma, or only commas".into(),
                ));
            }
            *c = '.';
        }
    }
    // A relative edit: `+=`, `-=`, `*=`, `/=` before the rest.
    let start = chars.iter().position(|c| !c.is_whitespace()).unwrap_or(0);
    let relative = match (chars.get(start), chars.get(start + 1)) {
        (Some(&op), Some('=')) if matches!(op, '+' | '-' | '\u{2212}' | '*' | '/') => {
            let op = if op == '\u{2212}' { '-' } else { op };
            Some((op, start + 2))
        }
        _ => None,
    };
    let from = relative.map_or(0, |(_, at)| at);
    let tokens = lex(&chars, &typed, from, kind).map_err(|(pos, what)| syntax(pos, what))?;
    let math = |what: &str| Error {
        pos: None,
        message: format!("Can't use '{shown}': {what}."),
    };
    let mut p = Parser {
        tokens: &tokens,
        at: 0,
        end: chars.len(),
        depth: 0,
        after: relative.map(|(op, _)| format!("{op}=")),
    };
    let v = p.expr().map_err(|e| match e {
        Fail::Syntax(pos, what) => syntax(pos, what),
        Fail::Math(what) => math(what),
    })?;
    if let Some(t) = tokens.get(p.at) {
        let what = match t.tok {
            Tok::Close => "there is one ) too many".to_string(),
            _ => format!("a + - * or / is missing before '{}'", t.text),
        };
        return Err(syntax(t.pos, what));
    }
    let v = match relative {
        None => v,
        Some((op, _)) => {
            let Some(cur) = current else {
                return Err(math(
                    "there is no single value to change here, so type the whole value",
                ));
            };
            match op {
                '+' => cur + v,
                '-' => cur - v,
                '*' => cur * v,
                _ if v == 0.0 => return Err(math("you can't divide by 0")),
                _ => cur / v,
            }
        }
    };
    if !v.is_finite() {
        return Err(math("the result is too large"));
    }
    Ok(if v == 0.0 { 0.0 } else { v })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Num(f64),
    Unit(f64),
    Plus,
    Minus,
    Times,
    Divide,
    Open,
    Close,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    /// Its first character.
    pos: usize,
    /// Its text, for messages.
    text: String,
}

/// The units of `kind`: their names and how many of the field's unit one
/// is; and the list for a message.
fn units(kind: Kind) -> (&'static [(&'static str, f64)], &'static str) {
    use crate::units::MM_PER_INCH;
    match kind {
        Kind::Plain => (&[], "this field takes no unit"),
        Kind::Length => (
            &[
                ("mm", 1.0),
                ("cm", 10.0),
                ("in", MM_PER_INCH),
                ("inch", MM_PER_INCH),
                ("\"", MM_PER_INCH),
            ],
            "use mm, cm or in",
        ),
        Kind::Angle => (&[("deg", 1.0), ("\u{00b0}", 1.0)], "use deg or \u{00b0}"),
        Kind::Percent => (&[("%", 1.0)], "use %"),
    }
}

/// Splits `chars[from..]` into tokens; an error is (position, problem).
/// `typed` is the text as typed (commas not yet dots), for messages.
fn lex(
    chars: &[char],
    typed: &[char],
    from: usize,
    kind: Kind,
) -> Result<Vec<Token>, (usize, String)> {
    let mut out = Vec::new();
    let mut i = from;
    let is_word = |c: char| c.is_alphabetic();
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        let simple = match c {
            '+' => Some(Tok::Plus),
            '-' | '\u{2212}' => Some(Tok::Minus),
            '*' | '\u{00d7}' => Some(Tok::Times),
            '/' | '\u{00f7}' => Some(Tok::Divide),
            '(' => Some(Tok::Open),
            ')' => Some(Tok::Close),
            _ => None,
        };
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if let Some(tok) = simple {
            out.push(Token {
                tok,
                pos: i,
                text: c.to_string(),
            });
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // An exponent: e, an optional sign, digits.
            if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                let mut j = i + 1;
                if j < chars.len() && matches!(chars[j], '+' | '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let text: String = chars[start..i].iter().collect();
            let mantissa = text.split(['e', 'E']).next().unwrap_or("");
            let dots = mantissa.matches('.').count();
            let digits = mantissa.chars().any(|c| c.is_ascii_digit());
            // Only digits, one dot and an exponent reach the parse, so
            // words like "inf" never do.
            let v = if dots <= 1 && digits {
                text.parse::<f64>().ok()
            } else {
                None
            };
            let as_typed: String = typed[start..i].iter().collect();
            let Some(v) = v else {
                return Err((start, format!("'{as_typed}' is not a number")));
            };
            out.push(Token {
                tok: Tok::Num(v),
                pos: start,
                text: as_typed,
            });
            continue;
        }
        let word: String = if is_word(c) {
            while i < chars.len() && is_word(chars[i]) {
                i += 1;
            }
            chars[start..i].iter().collect()
        } else if matches!(c, '"' | '\u{00b0}' | '%') {
            i += 1;
            c.to_string()
        } else {
            return Err((start, format!("'{c}' can't be used here")));
        };
        let (known, list) = units(kind);
        let lower = word.to_lowercase();
        let Some(&(_, factor)) = known.iter().find(|(n, _)| *n == lower) else {
            let what = if is_word(c) && !["mm", "cm", "in", "inch", "deg"].contains(&lower.as_str())
            {
                format!("'{word}' is not a unit here: {list}")
            } else {
                format!("{word} does not fit here: {list}")
            };
            return Err((start, what));
        };
        out.push(Token {
            tok: Tok::Unit(factor),
            pos: start,
            text: word,
        });
    }
    Ok(out)
}

enum Fail {
    Syntax(usize, String),
    Math(&'static str),
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    /// The length of the text, the position of "the end".
    end: usize,
    depth: usize,
    /// The relative edit in front, for "something is missing after +=".
    after: Option<String>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    /// The token before the current one, as text, for messages.
    fn before(&self) -> Option<String> {
        match self.at.checked_sub(1) {
            Some(i) => self.tokens.get(i).map(|t| t.text.clone()),
            None => self.after.clone(),
        }
    }

    fn finite(v: f64) -> Result<f64, Fail> {
        if v.is_finite() {
            Ok(v)
        } else {
            Err(Fail::Math("the result is too large"))
        }
    }

    fn expr(&mut self) -> Result<f64, Fail> {
        self.expr_unit().map(|(v, _)| v)
    }

    /// A sum, and whether a unit was typed in it.
    fn expr_unit(&mut self) -> Result<(f64, bool), Fail> {
        let (mut v, mut unit) = self.term()?;
        while let Some(t) = self.peek() {
            let plus = match t.tok {
                Tok::Plus => true,
                Tok::Minus => false,
                _ => break,
            };
            self.at += 1;
            let (r, ru) = self.term()?;
            v = Self::finite(if plus { v + r } else { v - r })?;
            unit |= ru;
        }
        Ok((v, unit))
    }

    /// Products and quotients; a unit counts for the product before it
    /// (`3/4in` is three quarters of an inch). Two values with units are
    /// not multiplied, and nothing is divided by a value with a unit.
    fn term(&mut self) -> Result<(f64, bool), Fail> {
        let (mut v, mut unit) = self.unary()?;
        while let Some(t) = self.peek() {
            let pos = t.pos;
            match t.tok {
                Tok::Times | Tok::Divide => {
                    let times = t.tok == Tok::Times;
                    self.at += 1;
                    let (r, ru) = self.unary()?;
                    if ru && (unit || !times) {
                        return Err(Self::units(pos));
                    }
                    if !times && r == 0.0 {
                        return Err(Fail::Math("you can't divide by 0"));
                    }
                    v = Self::finite(if times { v * r } else { v / r })?;
                    unit |= ru;
                }
                Tok::Unit(f) => {
                    if unit {
                        return Err(Self::units(pos));
                    }
                    self.at += 1;
                    v = Self::finite(v * f)?;
                    unit = true;
                }
                _ => break,
            }
        }
        Ok((v, unit))
    }

    fn units(pos: usize) -> Fail {
        Fail::Syntax(
            pos,
            "units can't be multiplied or divided by each other".into(),
        )
    }

    fn unary(&mut self) -> Result<(f64, bool), Fail> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            let pos = self.peek().map_or(self.end, |t| t.pos);
            return Err(Fail::Syntax(
                pos,
                "there are too many brackets or signs".into(),
            ));
        }
        let v = match self.peek().map(|t| t.tok) {
            Some(Tok::Minus) => {
                self.at += 1;
                let (v, u) = self.unary()?;
                (-v, u)
            }
            Some(Tok::Plus) => {
                self.at += 1;
                self.unary()?
            }
            _ => self.primary()?,
        };
        self.depth -= 1;
        Ok(v)
    }

    fn primary(&mut self) -> Result<(f64, bool), Fail> {
        let Some(t) = self.peek().cloned() else {
            let what = match self.before() {
                Some(b) => format!("something is missing after {b}"),
                None => "a number is missing".into(),
            };
            return Err(Fail::Syntax(self.end, what));
        };
        match t.tok {
            Tok::Num(v) => {
                self.at += 1;
                Ok((Self::finite(v)?, false))
            }
            Tok::Open => {
                self.at += 1;
                let v = self.expr_unit()?;
                match self.peek() {
                    Some(c) if c.tok == Tok::Close => {
                        self.at += 1;
                        Ok(v)
                    }
                    Some(c) => Err(Fail::Syntax(
                        c.pos,
                        format!("a + - * or / is missing before '{}'", c.text),
                    )),
                    None => Err(Fail::Syntax(self.end, "a ) is missing".into())),
                }
            }
            Tok::Unit(_) => Err(Fail::Syntax(
                t.pos,
                format!("a number is missing before {}", t.text),
            )),
            _ => {
                let what = match self.before() {
                    Some(b) => format!("something is missing after {b}"),
                    None => format!("a number is missing before {}", t.text),
                };
                Err(Fail::Syntax(t.pos, what))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn len(text: &str) -> f64 {
        eval(text, Kind::Length, None).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    fn plain(text: &str) -> f64 {
        eval(text, Kind::Plain, None).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    fn err(text: &str, kind: Kind) -> Error {
        match eval(text, kind, Some(10.0)) {
            Ok(v) => panic!("{text} gave {v}"),
            Err(e) => e,
        }
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn plain_numbers() {
        assert_eq!(plain("12"), 12.0);
        assert_eq!(plain("12.5"), 12.5);
        assert_eq!(plain(" 12.5 "), 12.5);
        assert_eq!(plain(".5"), 0.5);
        assert_eq!(plain("5."), 5.0);
        assert_eq!(plain("1e3"), 1000.0);
        assert_eq!(plain("1.5E-1"), 0.15);
        assert_eq!(plain("2e+2"), 200.0);
    }

    #[test]
    fn precedence() {
        assert_eq!(plain("2+3*4"), 14.0);
        assert_eq!(plain("2*3+4"), 10.0);
        assert_eq!(plain("10-4-3"), 3.0);
        assert_eq!(plain("100/10/2"), 5.0);
        assert!(near(plain("100/3"), 33.333333333333336));
        assert_eq!(plain("1 + 2 * 3 - 4 / 2"), 5.0);
    }

    #[test]
    fn brackets() {
        assert_eq!(plain("(2+3)*4"), 20.0);
        assert_eq!(plain("2*(3+4)"), 14.0);
        assert_eq!(plain("((1))"), 1.0);
        assert_eq!(plain("(1+(2*(3+1)))"), 9.0);
    }

    #[test]
    fn signs() {
        assert_eq!(plain("-5"), -5.0);
        assert_eq!(plain("+10"), 10.0);
        assert_eq!(plain("--5"), 5.0);
        assert_eq!(plain("3*-2"), -6.0);
        assert_eq!(plain("-(2+3)"), -5.0);
        assert_eq!(plain("2--3"), 5.0);
        // The typographic minus sign.
        assert_eq!(plain("\u{2212}4"), -4.0);
        assert_eq!(plain("6\u{2212}4"), 2.0);
        // Times and divided-by signs.
        assert_eq!(plain("6\u{00d7}4"), 24.0);
        assert_eq!(plain("6\u{00f7}4"), 1.5);
    }

    #[test]
    fn no_negative_zero() {
        let v = plain("-0");
        assert!(v == 0.0 && v.is_sign_positive());
        let v = Field::plain().eval("-0", None).unwrap();
        assert!(v.is_sign_positive());
    }

    #[test]
    fn comma_decimal_when_it_is_the_only_separator() {
        assert_eq!(plain("12,5"), 12.5);
        assert_eq!(plain("1,5 + 2,5"), 4.0);
        let e = err("1,5 + 2.5", Kind::Plain);
        assert_eq!(e.pos, Some(1));
        assert!(e.message.contains("dot"), "{e}");
        let e = err("1,2,3", Kind::Plain);
        assert!(e.message.contains("not a number"), "{e}");
    }

    #[test]
    fn length_units() {
        assert_eq!(len("10"), 10.0);
        assert_eq!(len("10mm"), 10.0);
        assert_eq!(len("1.5cm"), 15.0);
        assert!(near(len("2in"), 50.8));
        assert!(near(len("2 in"), 50.8));
        assert!(near(len("2\""), 50.8));
        assert!(near(len("2IN"), 50.8));
        assert!(near(len("1inch"), 25.4));
        assert!(near(len("-2in"), -50.8));
    }

    #[test]
    fn a_unit_counts_for_the_product_before_it() {
        assert!(near(len("3/4in"), 19.05));
        assert!(near(len("2*3in"), 152.4));
        assert!(near(len("1in*2"), 50.8));
        assert!(near(len("(1+1)in"), 50.8));
        assert!(near(len("2-3in"), 2.0 - 76.2));
    }

    #[test]
    fn units_are_not_multiplied_or_divided_by_each_other() {
        for (t, pos) in [
            ("2in*3cm", 5),
            ("1in/2cm", 5),
            ("2in mm", 4),
            ("(1in)*(2cm)", 5),
            ("10/(2in)", 2),
            ("(1in+1)in", 7),
        ] {
            let e = err(t, Kind::Length);
            assert!(
                e.message
                    .contains("units can't be multiplied or divided by each other"),
                "{t}: {e}"
            );
            assert_eq!(e.pos, Some(pos), "{t}");
        }
        // A unit after a product still counts for all of it.
        assert!(near(len("3/4in"), 19.05));
        assert!(near(len("1in*2 + 3cm/2"), 65.8));
    }

    #[test]
    fn a_comma_error_quotes_what_was_typed() {
        let e = err("1,2,3", Kind::Plain);
        assert!(e.message.contains("'1,2,3' is not a number"), "{e}");
        let e = err("1,5 3", Kind::Plain);
        assert!(e.message.contains("before '3'"), "{e}");
        let e = err("2 3,5", Kind::Plain);
        assert!(e.message.contains("before '3,5'"), "{e}");
    }

    #[test]
    fn a_very_long_text_is_refused_at_once() {
        let huge = "1".repeat(5_000_000);
        let e = eval(&huge, Kind::Plain, None).unwrap_err();
        assert!(e.message.contains("too long"), "{e}");
        assert!(e.message.contains("..."), "the message cuts it");
    }

    #[test]
    fn mixed_units() {
        assert!(near(len("10mm + 1in"), 35.4));
        assert!(near(len("1in - 1cm"), 15.4));
        assert!(near(len("10 + 1in"), 35.4));
        assert!(near(len("1cm + 1in + 1mm"), 36.4));
    }

    #[test]
    fn angle_and_percent_units() {
        assert_eq!(eval("90deg", Kind::Angle, None).unwrap(), 90.0);
        assert_eq!(eval("45\u{00b0}+45", Kind::Angle, None).unwrap(), 90.0);
        assert_eq!(eval("150%", Kind::Percent, None).unwrap(), 150.0);
    }

    #[test]
    fn a_unit_that_does_not_fit_is_refused() {
        let e = err("2in", Kind::Angle);
        assert_eq!(e.pos, Some(1));
        assert!(e.message.contains("deg"), "{e}");
        let e = err("2deg", Kind::Length);
        assert!(e.message.contains("mm, cm or in"), "{e}");
        let e = err("2mm", Kind::Plain);
        assert!(e.message.contains("no unit"), "{e}");
        let e = err("2ft", Kind::Length);
        assert!(e.message.contains("'ft'"), "{e}");
        let e = err("in", Kind::Length);
        assert_eq!(e.pos, Some(0));
    }

    #[test]
    fn relative_edits() {
        let f = |t: &str| eval(t, Kind::Length, Some(50.0)).unwrap();
        assert_eq!(f("+=10"), 60.0);
        assert_eq!(f("-=10"), 40.0);
        assert_eq!(f("*=2"), 100.0);
        assert_eq!(f("/=4"), 12.5);
        assert_eq!(f(" += 2*5 "), 60.0);
        assert!(near(f("+=1in"), 75.4));
        assert_eq!(f("+10"), 10.0, "a plain +10 is just 10");
        assert_eq!(f("\u{2212}=10"), 40.0);
    }

    #[test]
    fn a_relative_edit_needs_one_value() {
        let e = eval("+=10", Kind::Length, None).unwrap_err();
        assert!(e.message.contains("no single value"), "{e}");
        let e = eval("+=", Kind::Length, Some(1.0)).unwrap_err();
        assert!(e.message.contains("missing after +="), "{e}");
    }

    #[test]
    fn errors_name_the_problem_and_where() {
        let e = err("2in+", Kind::Length);
        assert_eq!(
            e.message,
            "Can't read '2in+': something is missing after +."
        );
        assert_eq!(e.pos, Some(4));
        let e = err("", Kind::Plain);
        assert!(e.message.contains("empty"), "{e}");
        let e = err("   ", Kind::Plain);
        assert!(e.message.contains("empty"), "{e}");
        let e = err("2 3", Kind::Plain);
        assert_eq!(e.pos, Some(2));
        assert!(e.message.contains("+ - * or /"), "{e}");
        let e = err("(2+3", Kind::Plain);
        assert_eq!(e.pos, Some(4));
        assert!(e.message.contains(")"), "{e}");
        let e = err("2+3)", Kind::Plain);
        assert_eq!(e.pos, Some(3));
        let e = err("2*/3", Kind::Plain);
        assert_eq!(e.pos, Some(2));
        let e = err("2#3", Kind::Plain);
        assert_eq!(e.pos, Some(1));
        assert!(e.message.contains("'#'"), "{e}");
        let e = err("1.2.3", Kind::Plain);
        assert_eq!(e.pos, Some(0));
        let e = err(".", Kind::Plain);
        assert_eq!(e.pos, Some(0));
        let e = err("()", Kind::Plain);
        assert_eq!(e.pos, Some(1));
        let e = err("abc", Kind::Plain);
        assert_eq!(e.pos, Some(0));
        // Positions count characters, not bytes.
        let e = err("45\u{00b0}+", Kind::Angle);
        assert_eq!(e.pos, Some(4));
    }

    #[test]
    fn not_a_number_words_are_refused() {
        for t in ["inf", "nan", "infinity", "NaN", "-inf", "1e", "e5"] {
            assert!(eval(t, Kind::Plain, None).is_err(), "{t}");
        }
    }

    #[test]
    fn division_by_zero_is_refused() {
        let e = err("5/0", Kind::Plain);
        assert!(e.message.contains("divide by 0"), "{e}");
        assert!(e.message.starts_with("Can't use '5/0'"), "{e}");
        assert!(err("5/(2-2)", Kind::Plain).message.contains("divide by 0"));
        assert!(err("/=0", Kind::Plain).message.contains("divide by 0"));
    }

    #[test]
    fn overflow_is_refused() {
        for t in [
            "1e309",
            "1e308*10",
            "1e200*1e200",
            "-1e308-1e308",
            "1e308/1e-10",
        ] {
            let e = err(t, Kind::Plain);
            assert!(e.message.contains("too large"), "{t}: {e}");
        }
    }

    #[test]
    fn deep_nesting_and_long_texts_are_refused_without_a_crash() {
        let deep = "(".repeat(100_000);
        assert!(eval(&deep, Kind::Plain, None).is_err());
        let signs = "-".repeat(150) + "1";
        assert!(eval(&signs, Kind::Plain, None).is_err());
        let nested = "(".repeat(60) + "1" + &")".repeat(60);
        assert_eq!(plain(&nested), 1.0);
        let long = "1+".repeat(150) + "1";
        let e = eval(&long, Kind::Plain, None).unwrap_err();
        assert!(e.message.contains("too long"), "{e}");
    }

    #[test]
    fn messages_have_no_em_dash_and_cut_long_texts() {
        let e = err("123456789012345678901234567890+", Kind::Plain);
        assert!(e.message.contains("12345678901234567890..."), "{e}");
        assert!(!e.message.contains('\u{2014}'));
    }

    #[test]
    fn the_field_checks_its_range() {
        let f = Field::length().range(0.0, 100.0);
        assert_eq!(f.eval("50", None), Ok(50.0));
        let e = f.eval("101", None).unwrap_err();
        assert_eq!(e.message, "101 is too large: give at most 100 mm.");
        let e = f.eval("-1", None).unwrap_err();
        assert_eq!(e.message, "-1 is too small: give at least 0 mm.");
        let e = Field::length().positive().eval("0", None).unwrap_err();
        assert!(e.message.contains("above 0"), "{e}");
        assert!(Field::length().positive().eval("1-2", None).is_err());
    }

    #[test]
    fn integer_fields_round_and_keep_their_limits() {
        let f = Field::plain().integer().range(1.0, 10.0);
        assert_eq!(f.eval("2.4", None), Ok(2.0));
        assert_eq!(f.eval("2.5", None), Ok(3.0));
        assert_eq!(f.eval("10/3", None), Ok(3.0));
        assert_eq!(f.eval("+=1", Some(4.0)), Ok(5.0));
        assert_eq!(f.eval("10.4", None), Ok(10.0));
        let e = f.eval("11", None).unwrap_err();
        assert_eq!(e.message, "11 is too large: give at most 10.");
        assert!(f.eval("0.4", None).is_err());
    }

    #[test]
    fn a_turn_is_reduced_into_one_turn() {
        let f = Field::turn();
        assert_eq!(f.eval("735", None), Ok(15.0));
        assert_eq!(f.eval("-90", None), Ok(270.0));
        assert_eq!(f.eval("1080", None), Ok(0.0));
        assert_eq!(f.eval("360", None), Ok(0.0));
        assert_eq!(f.eval("+=45", Some(330.0)), Ok(15.0));
        assert!(f.eval("-1e-20", None).unwrap() < 360.0);
        let v = f.eval("-0", None).unwrap();
        assert!(v == 0.0 && v.is_sign_positive());
        assert_eq!(f.eval("1e9", None), Ok(280.0));
        let e = f.eval("1e10", None).unwrap_err();
        assert!(e.message.contains("Type at most"), "{e}");
    }

    #[test]
    fn show_trims_numbers() {
        assert_eq!(show(10.0), "10");
        assert_eq!(show(0.25), "0.25");
        assert_eq!(show(1.0 / 3.0), "0.3333");
        assert_eq!(show(-0.0), "0");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2000))]

        #[test]
        fn never_panics_on_calculator_like_text(
            text in "[0-9 .,+*/()eE=\u{2212}\u{00d7}\u{00f7}\u{00b0}\"%a-z#-]{0,40}"
        ) {
            for kind in [Kind::Plain, Kind::Length, Kind::Angle, Kind::Percent] {
                if let Ok(v) = eval(&text, kind, Some(12.5)) {
                    prop_assert!(v.is_finite());
                }
                let _ = Field::turn().eval(&text, None);
                let _ = Field::plain().integer().range(1.0, 10.0).eval(&text, Some(3.0));
            }
        }

        #[test]
        fn never_panics_on_any_text(text in "\\PC{0,60}") {
            if let Ok(v) = eval(&text, Kind::Length, None) {
                prop_assert!(v.is_finite());
            }
        }

        #[test]
        fn sums_and_products_match(a in -1e6f64..1e6, b in -1e6f64..1e6) {
            let sa = format!("{a}");
            let sb = format!("({b})");
            let sum = eval(&format!("{sa}+{sb}"), Kind::Plain, None).unwrap();
            prop_assert!((sum - (a + b)).abs() <= 1e-9 * (a.abs() + b.abs()).max(1.0));
            let prod = eval(&format!("{sa}*{sb}"), Kind::Plain, None).unwrap();
            prop_assert!((prod - a * b).abs() <= 1e-9 * (a * b).abs().max(1.0));
        }
    }
}
