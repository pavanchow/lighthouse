//! The CSS parser and value model.
//!
//! A [`Stylesheet`] is a list of [`Rule`]s. Each rule has a list of selectors
//! and a list of declarations. Lighthouse supports simple selectors made of an
//! optional tag name, any number of classes and an optional id, for example
//! `div`, `.warning`, `#main` or `p.note`. The universal selector `*` is also
//! supported.
//!
//! Declarations are `name: value` pairs. Values are keywords (`block`, `auto`),
//! lengths in pixels (`10px`, `1.5px`), plain numbers, or colors (`#rgb`,
//! `#rrggbb`, `rgb(r,g,b)`, `rgba(r,g,b,a)`, and a handful of named colors).

/// A parsed stylesheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Stylesheet {
    /// Rules in source order. Source order is significant for the cascade.
    pub rules: Vec<Rule>,
}

/// A single CSS rule: `selectors { declarations }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// The comma separated selectors that share these declarations.
    pub selectors: Vec<Selector>,
    /// The property declarations.
    pub declarations: Vec<Declaration>,
}

/// A selector. Only simple selectors are modeled.
#[derive(Debug, Clone, PartialEq)]
pub enum Selector {
    /// A simple selector (tag, classes, id).
    Simple(SimpleSelector),
}

/// A simple selector: an optional tag, an optional id, and zero or more classes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SimpleSelector {
    /// Tag name, or `None` for the universal selector.
    pub tag_name: Option<String>,
    /// The id to match, if any.
    pub id: Option<String>,
    /// The class names that all must be present.
    pub class: Vec<String>,
}

/// A property declaration such as `width: 10px`.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    /// The property name, lowercased.
    pub name: String,
    /// The parsed value.
    pub value: Value,
}

/// A CSS value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A bare keyword such as `block` or `auto`.
    Keyword(String),
    /// A length with a unit.
    Length(f32, Unit),
    /// A plain unitless number.
    Number(f32),
    /// A color.
    ColorValue(Color),
}

/// A supported length unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    /// CSS pixels.
    Px,
}

/// An RGBA color, each channel 0 to 255.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel.
    pub a: u8,
}

/// Selector specificity as `(id, class, tag)`. Compared lexicographically, so
/// an id beats any number of classes and a class beats any number of tags.
pub type Specificity = (usize, usize, usize);

impl Selector {
    /// Compute the specificity of this selector.
    pub fn specificity(&self) -> Specificity {
        let Selector::Simple(simple) = self;
        let a = simple.id.iter().count();
        let b = simple.class.len();
        let c = simple.tag_name.iter().count();
        (a, b, c)
    }
}

impl Value {
    /// Return the length in pixels, treating any non length as zero.
    pub fn to_px(&self) -> f32 {
        match self {
            Value::Length(f, Unit::Px) => *f,
            Value::Number(f) => *f,
            _ => 0.0,
        }
    }
}

/// Parse a CSS source string into a [`Stylesheet`].
pub fn parse(source: &str) -> Stylesheet {
    let mut parser = Parser {
        input: source.as_bytes(),
        pos: 0,
    };
    Stylesheet {
        rules: parser.parse_rules(),
    }
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> u8 {
        self.input[self.pos]
    }

    fn parse_rules(&mut self) -> Vec<Rule> {
        let mut rules = Vec::new();
        loop {
            self.consume_whitespace_and_comments();
            if self.eof() {
                break;
            }
            if let Some(rule) = self.parse_rule() {
                rules.push(rule);
            }
        }
        rules
    }

    fn parse_rule(&mut self) -> Option<Rule> {
        let selectors = self.parse_selectors();
        let declarations = self.parse_declarations();
        if selectors.is_empty() {
            None
        } else {
            Some(Rule {
                selectors,
                declarations,
            })
        }
    }

    fn parse_selectors(&mut self) -> Vec<Selector> {
        let mut selectors = Vec::new();
        loop {
            self.consume_whitespace_and_comments();
            if self.eof() || self.peek() == b'{' {
                break;
            }
            selectors.push(Selector::Simple(self.parse_simple_selector()));
            self.consume_whitespace_and_comments();
            if !self.eof() && self.peek() == b',' {
                self.pos += 1;
            } else {
                break;
            }
        }
        // Highest specificity first is applied by the style stage, not here.
        selectors
    }

    fn parse_simple_selector(&mut self) -> SimpleSelector {
        let mut selector = SimpleSelector::default();
        while !self.eof() {
            match self.peek() {
                b'#' => {
                    self.pos += 1;
                    selector.id = Some(self.parse_identifier());
                }
                b'.' => {
                    self.pos += 1;
                    selector.class.push(self.parse_identifier());
                }
                b'*' => {
                    self.pos += 1;
                }
                c if is_identifier_char(c) => {
                    selector.tag_name = Some(self.parse_identifier().to_ascii_lowercase());
                }
                _ => break,
            }
        }
        selector
    }

    fn parse_declarations(&mut self) -> Vec<Declaration> {
        let mut declarations = Vec::new();
        self.consume_whitespace_and_comments();
        if self.eof() || self.peek() != b'{' {
            return declarations;
        }
        self.pos += 1; // '{'
        loop {
            self.consume_whitespace_and_comments();
            if self.eof() {
                break;
            }
            if self.peek() == b'}' {
                self.pos += 1;
                break;
            }
            if let Some(decl) = self.parse_declaration() {
                declarations.push(decl);
            }
            self.consume_whitespace_and_comments();
            if !self.eof() && self.peek() == b';' {
                self.pos += 1;
            }
        }
        declarations
    }

    fn parse_declaration(&mut self) -> Option<Declaration> {
        let name = self.parse_identifier().to_ascii_lowercase();
        self.consume_whitespace_and_comments();
        if self.eof() || self.peek() != b':' {
            // Skip to the next ';' or '}' to recover.
            self.skip_to_delimiter();
            return None;
        }
        self.pos += 1; // ':'
        self.consume_whitespace_and_comments();
        let value_str = self.consume_until(|c| c == b';' || c == b'}');
        if name.is_empty() {
            return None;
        }
        Some(Declaration {
            name,
            value: parse_value(value_str.trim()),
        })
    }

    fn skip_to_delimiter(&mut self) {
        while !self.eof() {
            let c = self.peek();
            if c == b';' || c == b'}' {
                break;
            }
            self.pos += 1;
        }
    }

    fn parse_identifier(&mut self) -> String {
        let start = self.pos;
        while !self.eof() && is_identifier_char(self.peek()) {
            self.pos += 1;
        }
        std::str::from_utf8(&self.input[start..self.pos])
            .unwrap_or("")
            .to_string()
    }

    fn consume_until<F: Fn(u8) -> bool>(&mut self, stop: F) -> String {
        let start = self.pos;
        while !self.eof() && !stop(self.peek()) {
            self.pos += 1;
        }
        std::str::from_utf8(&self.input[start..self.pos])
            .unwrap_or("")
            .to_string()
    }

    fn consume_whitespace_and_comments(&mut self) {
        loop {
            while !self.eof() && self.peek().is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.pos + 1 < self.input.len()
                && self.input[self.pos] == b'/'
                && self.input[self.pos + 1] == b'*'
            {
                self.pos += 2;
                while self.pos + 1 < self.input.len()
                    && !(self.input[self.pos] == b'*' && self.input[self.pos + 1] == b'/')
                {
                    self.pos += 1;
                }
                self.pos = (self.pos + 2).min(self.input.len());
            } else {
                break;
            }
        }
    }
}

fn is_identifier_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
}

/// Parse a single CSS value token (the text after `:` up to `;`).
pub fn parse_value(s: &str) -> Value {
    let s = s.trim();
    if let Some(color) = parse_color(s) {
        return Value::ColorValue(color);
    }
    if let Some(stripped) = s.strip_suffix("px") {
        if let Ok(n) = stripped.trim().parse::<f32>() {
            return Value::Length(n, Unit::Px);
        }
    }
    if let Ok(n) = s.parse::<f32>() {
        return Value::Number(n);
    }
    Value::Keyword(s.to_ascii_lowercase())
}

fn parse_color(s: &str) -> Option<Color> {
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    if let Some(inner) = s.strip_prefix("rgb(").and_then(|x| x.strip_suffix(')')) {
        return parse_rgb(inner, false);
    }
    if let Some(inner) = s.strip_prefix("rgba(").and_then(|x| x.strip_suffix(')')) {
        return parse_rgb(inner, true);
    }
    named_color(&s.to_ascii_lowercase())
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    match hex.len() {
        3 => {
            let r = dup_hex(&hex[0..1])?;
            let g = dup_hex(&hex[1..2])?;
            let b = dup_hex(&hex[2..3])?;
            Some(Color { r, g, b, a: 255 })
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color { r, g, b, a: 255 })
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Color { r, g, b, a })
        }
        _ => None,
    }
}

fn dup_hex(s: &str) -> Option<u8> {
    let v = u8::from_str_radix(s, 16).ok()?;
    Some(v * 17)
}

fn parse_rgb(inner: &str, alpha: bool) -> Option<Color> {
    let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
    let want = if alpha { 4 } else { 3 };
    if parts.len() != want {
        return None;
    }
    let r = parts[0].parse::<f32>().ok()?.clamp(0.0, 255.0) as u8;
    let g = parts[1].parse::<f32>().ok()?.clamp(0.0, 255.0) as u8;
    let b = parts[2].parse::<f32>().ok()?.clamp(0.0, 255.0) as u8;
    let a = if alpha {
        (parts[3].parse::<f32>().ok()?.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        255
    };
    Some(Color { r, g, b, a })
}

fn named_color(name: &str) -> Option<Color> {
    let c = |r, g, b| Some(Color { r, g, b, a: 255 });
    match name {
        "black" => c(0, 0, 0),
        "white" => c(255, 255, 255),
        "red" => c(255, 0, 0),
        "green" => c(0, 128, 0),
        "lime" => c(0, 255, 0),
        "blue" => c(0, 0, 255),
        "yellow" => c(255, 255, 0),
        "cyan" | "aqua" => c(0, 255, 255),
        "magenta" | "fuchsia" => c(255, 0, 255),
        "gray" | "grey" => c(128, 128, 128),
        "silver" => c(192, 192, 192),
        "maroon" => c(128, 0, 0),
        "olive" => c(128, 128, 0),
        "navy" => c(0, 0, 128),
        "teal" => c(0, 128, 128),
        "purple" => c(128, 0, 128),
        "orange" => c(255, 165, 0),
        "transparent" => Some(Color { r: 0, g: 0, b: 0, a: 0 }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_length_number_and_keyword() {
        assert_eq!(parse_value("10px"), Value::Length(10.0, Unit::Px));
        assert_eq!(parse_value("1.5"), Value::Number(1.5));
        assert_eq!(parse_value("Block"), Value::Keyword("block".to_string()));
    }

    #[test]
    fn parses_hex_and_named_colors() {
        assert_eq!(parse_value("#fff"), Value::ColorValue(Color { r: 255, g: 255, b: 255, a: 255 }));
        assert_eq!(parse_value("#ff0000"), Value::ColorValue(Color { r: 255, g: 0, b: 0, a: 255 }));
        assert_eq!(parse_value("rgb(0, 128, 255)"), Value::ColorValue(Color { r: 0, g: 128, b: 255, a: 255 }));
        assert_eq!(parse_value("navy"), Value::ColorValue(Color { r: 0, g: 0, b: 128, a: 255 }));
    }

    #[test]
    fn parses_a_rule_with_multiple_selectors() {
        let sheet = parse("h1, .title { color: red; margin: 4px; }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selectors.len(), 2);
        assert_eq!(sheet.rules[0].declarations.len(), 2);
    }

    #[test]
    fn ignores_comments() {
        let sheet = parse("/* c */ div { width: 5px; } /* trailing */");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn specificity_orders_id_class_tag() {
        let sheet = parse("#a {} .b {} div {}");
        assert_eq!(sheet.rules[0].selectors[0].specificity(), (1, 0, 0));
        assert_eq!(sheet.rules[1].selectors[0].specificity(), (0, 1, 0));
        assert_eq!(sheet.rules[2].selectors[0].specificity(), (0, 0, 1));
    }
}
