//! Lighthouse is a from scratch browser rendering engine with zero external
//! dependencies. It implements the full core pipeline of a browser rendering
//! engine so that each stage can be inspected on its own:
//!
//! ```text
//! HTML text  -> [html]   -> DOM tree
//! CSS text   -> [css]    -> Stylesheet
//! DOM + CSS  -> [style]  -> Styled tree (computed values)
//! Styled tree-> [layout] -> Layout tree (absolute rectangles)
//! Layout tree-> [paint]  -> Display list + raster
//! ```
//!
//! Every stage is a standalone module with its own data types, so a program (or
//! an agent) can call one stage, look at the intermediate representation, then
//! feed it into the next. See `DESIGN.md` for the algorithms and the documented
//! HTML and CSS subset.

#![warn(clippy::pedantic)]
// A rendering engine converts between floats and integer pixel or byte
// coordinates constantly, so the cast lints are expected and intentional. Exact
// float comparisons are used deliberately in golden layout checks, and the small
// pure helpers do not benefit from `#[must_use]` noise.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_lossless,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::similar_names,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::missing_panics_doc
)]

pub mod css;
pub mod dom;
pub mod fuzz;
pub mod html;
pub mod layout;
pub mod paint;
pub mod style;

/// Parse an HTML source string into a DOM tree.
pub fn parse_html(source: &str) -> dom::Node {
    html::parse(source)
}

/// Parse a CSS source string into a stylesheet.
pub fn parse_css(source: &str) -> css::Stylesheet {
    css::parse(source)
}
