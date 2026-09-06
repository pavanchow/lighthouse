//! The layout engine: turn a styled tree into a tree of boxes with absolute
//! rectangles.
//!
//! Lighthouse implements the CSS box model for block boxes plus a simple inline
//! text flow. The algorithm follows the classic two pass block layout:
//!
//! - Widths are resolved top down. A block box fills its containing block width,
//!   resolving `auto` widths and `auto` margins, honoring padding and borders.
//! - Positions and heights are resolved as children are laid out. Block siblings
//!   stack vertically, each starting below the previous sibling's margin box.
//!   A block with `height: auto` grows to contain its children.
//!
//! Inline content (text and inline elements) inside a block is laid out into
//! line boxes: inline boxes are placed left to right and wrap to a new line when
//! they would exceed the content width. Text width is estimated from font size,
//! which is enough to demonstrate real line breaking without a font rasterizer.
//!
//! When a block contains a mix of block and inline children, the inline runs are
//! wrapped in anonymous block boxes so the block formatting context stays clean.

use crate::css::{Unit, Value};
use crate::style::{Display, StyledNode};
use std::fmt::Write as _;

/// A rectangle in absolute device coordinates. The origin is the top left.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Rect {
    /// Expand this rectangle by the given edge sizes on all four sides.
    pub fn expanded_by(self, edge: EdgeSizes) -> Rect {
        Rect {
            x: self.x - edge.left,
            y: self.y - edge.top,
            width: self.width + edge.left + edge.right,
            height: self.height + edge.top + edge.bottom,
        }
    }
}

/// The four edge sizes around a box, used for margin, border and padding.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeSizes {
    /// Left edge size.
    pub left: f32,
    /// Right edge size.
    pub right: f32,
    /// Top edge size.
    pub top: f32,
    /// Bottom edge size.
    pub bottom: f32,
}

/// The full box model dimensions of a laid out box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Dimensions {
    /// The content box, in absolute coordinates.
    pub content: Rect,
    /// Padding around the content.
    pub padding: EdgeSizes,
    /// Border around the padding.
    pub border: EdgeSizes,
    /// Margin around the border.
    pub margin: EdgeSizes,
}

impl Dimensions {
    /// The content plus padding.
    pub fn padding_box(self) -> Rect {
        self.content.expanded_by(self.padding)
    }
    /// The padding box plus border.
    pub fn border_box(self) -> Rect {
        self.padding_box().expanded_by(self.border)
    }
    /// The border box plus margin.
    pub fn margin_box(self) -> Rect {
        self.border_box().expanded_by(self.margin)
    }
}

/// The kind of a layout box.
#[derive(Debug, Clone)]
pub enum BoxType<'a> {
    /// A block level box for a styled node.
    BlockNode(&'a StyledNode<'a>),
    /// An inline level box for a styled node.
    InlineNode(&'a StyledNode<'a>),
    /// An anonymous block box that groups inline content.
    AnonymousBlock,
}

/// A node in the layout tree.
#[derive(Debug, Clone)]
pub struct LayoutBox<'a> {
    /// The absolute geometry of this box.
    pub dimensions: Dimensions,
    /// What kind of box this is.
    pub box_type: BoxType<'a>,
    /// Child boxes.
    pub children: Vec<LayoutBox<'a>>,
}

impl<'a> LayoutBox<'a> {
    fn new(box_type: BoxType<'a>) -> Self {
        LayoutBox {
            dimensions: Dimensions::default(),
            box_type,
            children: Vec::new(),
        }
    }

    fn styled_node(&self) -> Option<&'a StyledNode<'a>> {
        match self.box_type {
            BoxType::BlockNode(n) | BoxType::InlineNode(n) => Some(n),
            BoxType::AnonymousBlock => None,
        }
    }

    /// Where should the next inline child go: an existing anonymous block, or a
    /// new one.
    fn get_inline_container(&mut self) -> &mut LayoutBox<'a> {
        match self.box_type {
            BoxType::InlineNode(_) | BoxType::AnonymousBlock => self,
            BoxType::BlockNode(_) => {
                let needs_new = !matches!(
                    self.children.last().map(|c| &c.box_type),
                    Some(BoxType::AnonymousBlock)
                );
                if needs_new {
                    self.children.push(LayoutBox::new(BoxType::AnonymousBlock));
                }
                self.children.last_mut().unwrap()
            }
        }
    }
}

/// Lay out a styled tree inside a viewport and return the layout tree.
///
/// The viewport width bounds the initial containing block. Its height starts at
/// zero and grows as content is laid out.
pub fn layout_tree<'a>(node: &'a StyledNode<'a>, mut viewport: Dimensions) -> LayoutBox<'a> {
    viewport.content.height = 0.0;
    let mut root = build_layout_tree(node);
    root.layout(viewport);
    root
}

/// Build the box tree from the styled tree, choosing box types from `display`
/// and skipping `display: none` subtrees.
pub fn build_layout_tree<'a>(styled: &'a StyledNode<'a>) -> LayoutBox<'a> {
    let mut root = LayoutBox::new(match styled.display() {
        Display::Inline => BoxType::InlineNode(styled),
        Display::Block | Display::None => BoxType::BlockNode(styled),
    });

    // In a block formatting context (a node with at least one block child),
    // whitespace only text between blocks is not rendered, matching browsers.
    let block_context = styled
        .children
        .iter()
        .any(|c| c.display() == Display::Block);

    for child in &styled.children {
        match child.display() {
            Display::Block => root.children.push(build_layout_tree(child)),
            Display::Inline => {
                if block_context && is_whitespace_text(child) {
                    continue;
                }
                root.get_inline_container()
                    .children
                    .push(build_layout_tree(child));
            }
            Display::None => {}
        }
    }
    root
}

fn is_whitespace_text(node: &StyledNode) -> bool {
    matches!(&node.node.node_type, crate::dom::NodeType::Text(t) if t.trim().is_empty())
}

const DEFAULT_FONT_SIZE: f32 = 16.0;
const CHAR_WIDTH_RATIO: f32 = 0.5;
const LINE_HEIGHT_RATIO: f32 = 1.2;

impl LayoutBox<'_> {
    fn layout(&mut self, containing_block: Dimensions) {
        // Every box type currently uses the block layout path. Inline flow is
        // handled inside `layout_children` for boxes with inline content.
        self.layout_block(containing_block);
    }

    fn layout_block(&mut self, containing_block: Dimensions) {
        self.calculate_block_width(containing_block);
        self.calculate_block_position(containing_block);
        self.layout_children();
        self.calculate_block_height();
    }

    fn calculate_block_width(&mut self, containing_block: Dimensions) {
        let style = self.styled_node();
        let auto = Value::Keyword("auto".to_string());
        let zero = Value::Length(0.0, Unit::Px);

        let get = |name: &str, fallback: &str, default: &Value| match style {
            Some(s) => s.lookup(name, fallback, default),
            None => default.clone(),
        };

        let mut width = get("width", "width", &auto);

        let margin_left = get("margin-left", "margin", &zero);
        let margin_right = get("margin-right", "margin", &zero);
        let border_left = get("border-left-width", "border-width", &zero);
        let border_right = get("border-right-width", "border-width", &zero);
        let padding_left = get("padding-left", "padding", &zero);
        let padding_right = get("padding-right", "padding", &zero);

        let total: f32 = [
            &margin_left,
            &margin_right,
            &border_left,
            &border_right,
            &padding_left,
            &padding_right,
            &width,
        ]
        .iter()
        .map(|v| v.to_px())
        .sum();

        let mut margin_left = margin_left;

        let container_width = containing_block.content.width;
        if width != auto && margin_left == auto && total > container_width {
            margin_left = zero.clone();
        }

        let underflow = container_width - total;
        let width_is_auto = width == auto;
        let ml_is_auto = margin_left == auto;
        let mr_is_auto = margin_right == auto;

        // Resolve `auto` width and `auto` left margin. The right margin only ever
        // absorbs slack, which the containment clip below computes directly, so it
        // is never assigned here.
        match (width_is_auto, ml_is_auto, mr_is_auto) {
            (false, true, false) => margin_left = Value::Length(underflow, Unit::Px),
            (false, true, true) => margin_left = Value::Length(underflow / 2.0, Unit::Px),
            (true, _, _) => {
                if ml_is_auto {
                    margin_left = zero.clone();
                }
                width = if underflow >= 0.0 {
                    Value::Length(underflow, Unit::Px)
                } else {
                    zero.clone()
                };
            }
            _ => {}
        }

        let mut content_w = width.to_px();

        // `box-sizing: border-box`: the specified width covers content, padding
        // and border, so subtract them to recover the content width.
        if border_box_sizing(style) {
            content_w = (content_w
                - padding_left.to_px()
                - padding_right.to_px()
                - border_left.to_px()
                - border_right.to_px())
            .max(0.0);
        }

        // Containment clip, the documented overflow behavior. An in-flow block
        // box is clipped so its border box can never escape its containing block.
        // The horizontal extent (left margin, borders, padding, content) is
        // distributed within the container width in that fixed priority order,
        // and the right margin absorbs any slack. Every value stays non-negative,
        // so containment holds even when the specified box is far too wide.
        let mut remaining = container_width.max(0.0);
        let take = |remaining: &mut f32, want: f32| {
            let got = want.max(0.0).min(*remaining);
            *remaining -= got;
            got
        };
        let ml = take(&mut remaining, margin_left.to_px());
        let border_l = take(&mut remaining, border_left.to_px());
        let border_r = take(&mut remaining, border_right.to_px());
        let pad_l = take(&mut remaining, padding_left.to_px());
        let pad_r = take(&mut remaining, padding_right.to_px());
        content_w = take(&mut remaining, content_w);
        let mr = remaining.max(0.0);

        let d = &mut self.dimensions;
        d.content.width = content_w;
        d.padding.left = pad_l;
        d.padding.right = pad_r;
        d.border.left = border_l;
        d.border.right = border_r;
        d.margin.left = ml;
        d.margin.right = mr;
    }

    fn calculate_block_position(&mut self, containing_block: Dimensions) {
        let style = self.styled_node();
        let zero = Value::Length(0.0, Unit::Px);
        let get = |name: &str, fallback: &str| match style {
            Some(s) => s.lookup(name, fallback, &zero),
            None => zero.clone(),
        };

        let d = &mut self.dimensions;
        d.margin.top = get("margin-top", "margin").to_px();
        d.margin.bottom = get("margin-bottom", "margin").to_px();
        d.border.top = get("border-top-width", "border-width").to_px();
        d.border.bottom = get("border-bottom-width", "border-width").to_px();
        d.padding.top = get("padding-top", "padding").to_px();
        d.padding.bottom = get("padding-bottom", "padding").to_px();

        d.content.x = containing_block.content.x + d.margin.left + d.border.left + d.padding.left;
        d.content.y = containing_block.content.height
            + containing_block.content.y
            + d.margin.top
            + d.border.top
            + d.padding.top;
    }

    fn layout_children(&mut self) {
        // Inline children (text and inline boxes) get line based flow. Any box
        // whose children are inline nodes is treated as an inline formatting
        // context. Anonymous blocks always hold inline content.
        if self.has_inline_children() {
            self.layout_inline_children();
        } else {
            let d = &mut self.dimensions;
            for child in &mut self.children {
                child.layout(*d);
                d.content.height += child.dimensions.margin_box().height;
            }
        }
    }

    fn has_inline_children(&self) -> bool {
        matches!(self.box_type, BoxType::AnonymousBlock)
            || (!self.children.is_empty()
                && self
                    .children
                    .iter()
                    .all(|c| matches!(c.box_type, BoxType::InlineNode(_))))
            || self.is_text_leaf()
    }

    fn is_text_leaf(&self) -> bool {
        self.children.is_empty()
            && matches!(self.box_type, BoxType::InlineNode(n) if is_text(n))
    }

    fn font_size(&self) -> f32 {
        match self.styled_node().and_then(|s| s.value("font-size")) {
            Some(v) => {
                let px = v.to_px();
                if px > 0.0 {
                    px
                } else {
                    DEFAULT_FONT_SIZE
                }
            }
            None => DEFAULT_FONT_SIZE,
        }
    }

    fn layout_inline_children(&mut self) {
        let content_width = self.dimensions.content.width;
        let origin_x = self.dimensions.content.x;
        let origin_y = self.dimensions.content.y;

        let mut cursor_x = 0.0f32;
        let mut line_top = 0.0f32;
        let mut line_height = 0.0f32;
        let mut max_line_width = 0.0f32;

        // A leaf inline box that directly carries text has no inline children,
        // so give it a single line sized to its text.
        if self.is_text_leaf() {
            let fs = self.font_size();
            let text_w = inline_text_width(self.styled_node().unwrap(), fs).min(content_width.max(fs));
            self.dimensions.content.width = if content_width > 0.0 {
                content_width
            } else {
                text_w
            };
            self.dimensions.content.height = fs * LINE_HEIGHT_RATIO;
            return;
        }

        let mut boxes = std::mem::take(&mut self.children);
        for child in &mut boxes {
            let fs = child.font_size();
            let width = inline_box_width(child, fs);
            let height = fs * LINE_HEIGHT_RATIO;

            if cursor_x > 0.0 && cursor_x + width > content_width {
                // Wrap to a new line.
                max_line_width = max_line_width.max(cursor_x);
                line_top += line_height;
                cursor_x = 0.0;
                line_height = 0.0;
            }

            let d = &mut child.dimensions;
            d.content.x = origin_x + cursor_x;
            d.content.y = origin_y + line_top;
            d.content.width = width;
            d.content.height = height;
            child.position_inline_text();

            cursor_x += width;
            line_height = line_height.max(height);
        }
        max_line_width = max_line_width.max(cursor_x);

        self.children = boxes;
        self.dimensions.content.height = line_top + line_height;
        if content_width <= 0.0 {
            self.dimensions.content.width = max_line_width;
        }
    }

    /// Give an inline element's own inline children absolute positions inside it.
    fn position_inline_text(&mut self) {
        let origin_x = self.dimensions.content.x;
        let origin_y = self.dimensions.content.y;
        let mut cursor_x = 0.0f32;
        for child in &mut self.children {
            let fs = child.font_size();
            let width = inline_box_width(child, fs);
            child.dimensions.content.x = origin_x + cursor_x;
            child.dimensions.content.y = origin_y;
            child.dimensions.content.width = width;
            child.dimensions.content.height = fs * LINE_HEIGHT_RATIO;
            child.position_inline_text();
            cursor_x += width;
        }
    }

    fn calculate_block_height(&mut self) {
        // Respect an explicit height, otherwise keep the height accumulated
        // while laying out children.
        if let Some(style) = self.styled_node() {
            if let Some(Value::Length(h, Unit::Px)) = style.value("height") {
                let mut h = h;
                if border_box_sizing(Some(style)) {
                    let d = &self.dimensions;
                    h -= d.padding.top + d.padding.bottom + d.border.top + d.border.bottom;
                }
                self.dimensions.content.height = h.max(0.0);
            }
        }
    }
}

/// Whether the box uses `box-sizing: border-box`, where the specified width and
/// height include padding and border. The default is `content-box`.
fn border_box_sizing(style: Option<&StyledNode>) -> bool {
    matches!(
        style.and_then(|s| s.value("box-sizing")),
        Some(Value::Keyword(k)) if k == "border-box"
    )
}

fn is_text(node: &StyledNode) -> bool {
    matches!(node.node.node_type, crate::dom::NodeType::Text(_))
}

fn inline_box_width(b: &LayoutBox, font_size: f32) -> f32 {
    match b.box_type {
        BoxType::InlineNode(n) => inline_text_width(n, font_size),
        _ => 0.0,
    }
}

fn inline_text_width(node: &StyledNode, font_size: f32) -> f32 {
    match &node.node.node_type {
        crate::dom::NodeType::Text(data) => data.trim().chars().count() as f32 * font_size * CHAR_WIDTH_RATIO,
        crate::dom::NodeType::Element(_) => node
            .children
            .iter()
            .map(|c| inline_text_width(c, font_size))
            .sum(),
    }
}

/// Pretty print a layout tree, one box per line with its content rectangle.
pub fn pretty(root: &LayoutBox) -> String {
    let mut out = String::new();
    pretty_into(root, &mut out, 0);
    out
}

fn pretty_into(b: &LayoutBox, out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    let label = match &b.box_type {
        BoxType::BlockNode(n) => format!("block {}", node_label(n)),
        BoxType::InlineNode(n) => format!("inline {}", node_label(n)),
        BoxType::AnonymousBlock => "anon-block".to_string(),
    };
    let c = b.dimensions.content;
    let _ = writeln!(
        out,
        "{label}  content=(x:{:.1} y:{:.1} w:{:.1} h:{:.1})",
        c.x, c.y, c.width, c.height
    );
    for child in &b.children {
        pretty_into(child, out, depth + 1);
    }
}

fn node_label(n: &StyledNode) -> String {
    match &n.node.node_type {
        crate::dom::NodeType::Element(e) => format!("<{}>", e.tag_name),
        crate::dom::NodeType::Text(t) => format!("#text \"{}\"", t.trim()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css;
    use crate::html;
    use crate::style::style_tree;

    fn viewport(width: f32) -> Dimensions {
        Dimensions {
            content: Rect {
                x: 0.0,
                y: 0.0,
                width,
                height: 0.0,
            },
            ..Default::default()
        }
    }

    #[test]
    fn edge_expansion_grows_rect() {
        let r = Rect {
            x: 10.0,
            y: 10.0,
            width: 100.0,
            height: 50.0,
        };
        let e = EdgeSizes {
            left: 5.0,
            right: 5.0,
            top: 2.0,
            bottom: 3.0,
        };
        let out = r.expanded_by(e);
        assert_eq!(out.x, 5.0);
        assert_eq!(out.width, 110.0);
        assert_eq!(out.height, 55.0);
    }

    #[test]
    fn auto_width_fills_containing_block() {
        let dom = html::parse("<div>x</div>");
        let sheet = css::parse("div { display: block; }");
        let styled = style_tree(&dom, &sheet);
        let layout = layout_tree(&styled, viewport(500.0));
        assert_eq!(layout.children[0].dimensions.content.width, 500.0);
    }

    #[test]
    fn display_none_is_skipped_in_box_tree() {
        let dom = html::parse("<div class=a>a</div><div class=b>b</div>");
        let sheet = css::parse("div { display: block; } .a { display: none; }");
        let styled = style_tree(&dom, &sheet);
        let layout = layout_tree(&styled, viewport(400.0));
        assert_eq!(layout.children.len(), 1);
    }
}
