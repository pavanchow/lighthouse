# Lighthouse design

This document describes the architecture of Lighthouse, the algorithm in each stage, and the documented HTML and CSS subset. The guiding principle is that every stage is a standalone module with its own data types, so each intermediate representation can be printed, tested, and fed into the next stage on its own.

## Architecture

The pipeline is a straight line of pure transforms.

```text
source HTML --> html::parse    --> dom::Node          (the DOM tree)
source CSS  --> css::parse     --> css::Stylesheet    (rules and selectors)
DOM, sheet  --> style::style_tree --> StyledNode      (computed values)
StyledNode  --> layout::layout_tree --> LayoutBox     (absolute rectangles)
LayoutBox   --> paint::build_display_list --> DisplayList --> Canvas raster
```

Each module owns its types. `dom` owns `Node` and `ElementData`. `css` owns `Stylesheet`, `Selector`, and `Value`. `style` owns `StyledNode`. `layout` owns `LayoutBox` and `Dimensions`. `paint` owns `DisplayCommand` and `Canvas`. No stage reaches back into a previous stage's internals, it only reads the published data type. That is what makes the engine inspectable rather than a black box.

## The HTML parser

Parsing runs in two phases, exactly like a real engine.

### Tokenizer

`html::Tokenizer` scans the source bytes and produces a flat stream of `Token` values: start tags (with attributes and a self closing flag), end tags, text runs, comments, and the doctype. The scanner always makes forward progress, so malformed input cannot loop.

Attribute values may be double quoted, single quoted, or unquoted, and boolean attributes with no value are supported. Text and quoted attribute values are entity decoded during tokenization.

### Tree builder

`html::TreeBuilder` consumes the token stream and maintains a stack of open elements. A synthetic `__root__` element sits at the bottom of the stack so a fragment with several top level nodes still yields a single tree. The serializer knows about `__root__` and emits only its children, so it never appears in serialized output.

The tree builder applies these rules:

- **Void elements** (`area`, `base`, `br`, `col`, `embed`, `hr`, `img`, `input`, `link`, `meta`, `param`, `source`, `track`, `wbr`) never take children and never push onto the open stack. An end tag for a void element is ignored.
- **Self closing syntax** (`<tag/>`) closes the element immediately for any tag.
- **Implicit closing** handles the common cases where authors omit end tags. A new `li` closes an open `li`. A new `option` closes an open `option`. A new `tr` closes an open `tr`, `td`, or `th`. A new `td` or `th` closes an open `td` or `th`. A new `p`, or any block level start tag, closes an open `p`.
- **End tags** close the nearest matching open element, popping any unclosed elements in between. An end tag with no matching open element is ignored.
- Comments and the doctype are dropped, matching how they are ignored for layout.

This is a documented subset, not the full HTML5 error recovery algorithm. It covers well formed documents and the most common author shortcuts, which is enough to drive the rest of the pipeline.

### Round trip

`Node::to_html` serializes a tree back to HTML, entity encoding text and attribute values and omitting end tags for void elements. Because serialization is the inverse of parsing for the supported subset, `parse(serialize(parse(x)))` equals `parse(x)`. Gate 1 asserts exactly this.

## The CSS cascade and specificity

`css::parse` produces a `Stylesheet`, a list of `Rule` values. Each rule has a list of comma separated selectors and a list of declarations.

### Selectors

Only simple selectors are modeled: an optional tag name, an optional id, and any number of classes. Examples are `div`, `.warning`, `#main`, `p.note`, and the universal selector `*`. A selector matches an element when its tag (if present) equals the element tag, its id (if present) equals the element id, and every class it names is present on the element.

### Values

A `Value` is a keyword (`block`, `auto`), a length in pixels (`10px`), a plain number, or a color. Colors accept `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(r, g, b)`, `rgba(r, g, b, a)`, and a set of named colors.

### Specificity

Specificity is the triple `(id count, class count, tag count)` and it is compared lexicographically. An id therefore beats any number of classes, and a class beats any number of tags. This is the standard CSS rule for the supported selector set. Gate 2 checks the calculation directly against hand computed values.

### The cascade

For one element, `style` collects every matching rule together with the specificity that matched, then sorts the matches by ascending specificity. Because the sort is stable, matches of equal specificity keep their source order. Applying the sorted declarations front to back leaves the strongest declaration last, so it wins. The result is: highest specificity wins, and ties are broken by latest source order. Gate 2 verifies both halves of that rule.

## Style resolution

`style::style_tree` walks the DOM and produces a parallel `StyledNode` tree. Each styled node carries a map of computed property values from the cascade.

A small set of properties inherit from parent to child when the child does not set them: `color`, `font-size`, `font-family`, `line-height`, and `text-align`. Non inherited properties such as `width` do not leak to descendants. Inheritance is applied by seeding a child's value map with the inheritable subset of its parent's values before the child's own matched rules are applied.

The outer `display` type is resolved here. An explicit `display` declaration always wins. Otherwise a small user agent default applies: structural elements (`html`, `body`, `div`, `p`, headings, `li`, `ul`, `ol`, `section`, and similar) default to block, metadata elements (`head`, `style`, `script`, `title`, `meta`, `link`) default to none so their contents are not laid out, and everything else defaults to inline. This mirrors a browser rendering a page with no author stylesheet.

## The layout algorithm

`layout::layout_tree` first builds a box tree from the styled tree, then lays it out inside a viewport, producing a `LayoutBox` tree where every box has absolute rectangles.

### Box generation

Each styled node becomes a block box, an inline box, or nothing (`display: none` subtrees are dropped). When a block box has a mix of block and inline children, the inline runs are wrapped in anonymous block boxes so the block formatting context stays clean. In a block formatting context, whitespace only text between blocks is dropped, matching browser behavior.

### The box model

Every box has content, padding, border, and margin, held in `Dimensions`. The padding box is content plus padding, the border box adds border, and the margin box adds margin. Helpers expand a rectangle by edge sizes to move between these boxes.

### Block layout

Block layout is the classic two pass algorithm.

- **Width** is resolved top down. A block fills its containing block width. The engine sums margins, borders, padding, and width, computes the underflow against the container, and distributes it. An `auto` width absorbs the underflow. `auto` margins center or align the box. If the box is over constrained, `auto` margins are treated as zero and width clamps at zero rather than going negative.
- **Position** places the content box using the containing block origin plus the left and top margin, border, and padding. The vertical position starts below the content already laid out in the container, so block siblings stack.
- **Children** are laid out in order. After each child, the container content height grows by that child's margin box height, which is how an `auto` height block grows to contain its children. An explicit `height` overrides this.

### Inline layout

When a block establishes an inline formatting context, its inline children flow into line boxes. Inline boxes are placed left to right. When the next box would exceed the content width, the cursor wraps to a new line and the vertical position advances by the line height. Text width is estimated from the font size (half the font size per character) and line height is a fixed multiple of the font size. This gives real line breaking without a font rasterizer, which is enough to show inline flow and wrapping while keeping the engine dependency free.

## Paint

`paint::build_display_list` walks the layout tree and emits a flat, ordered `DisplayList`. For each box it emits a background rectangle, four border edge rectangles, and text commands. Separating the display list from the drawing is how real engines feed a rasterizer or a GPU.

`paint::Canvas` is a software raster surface of RGBA pixels. It fills rectangles with alpha blending and approximates text as a faint band, since there is no glyph rasterizer. `Canvas::to_ascii` samples the raster into a character grid by average luminance, which is the headless verification surface. It lets the CLI and tests show painted output with no windowing system.

## Why each gate proves its claim

- **Gate 1, HTML parse correctness.** Direct structural assertions prove the tree shape, attributes, void handling, and implicit closing are correct on known documents. The round trip proves serialization is a true inverse of parsing, which catches any lossy or reordering bug because a mismatch changes the reparsed tree.
- **Gate 2, cascade and specificity.** A reference specificity calculation proves the ordering rule in isolation. Documents matched by several competing rules prove the full cascade picks the right winner by specificity and then by source order, including the case where a later low specificity rule must lose to an earlier high specificity one.
- **Gate 3, layout invariants.** The randomized generator explores many tree shapes, and the invariants (containment, non overlapping stacking, non negative dimensions) are properties that must hold for any correct block layout, so a violation on any generated tree is a real bug. Golden tests then pin exact rectangles that were computed by hand, which catches errors that still satisfy the invariants, such as an off by a margin position.

## Documented HTML and CSS subset

HTML supported: elements and text, attributes (double, single, unquoted, and boolean), void elements, self closing syntax, comments and doctype (dropped), entity decoding for `&amp; &lt; &gt; &quot; &apos; &nbsp;` and numeric `&#NN;` and `&#xHH;`, and the implicit closing rules listed above. Not supported: the full HTML5 error recovery state machine, raw text elements beyond simple handling, templates, and namespaced foreign content.

CSS supported: rules with comma separated simple selectors (tag, id, classes, universal), declarations, comments, lengths in pixels, plain numbers, keywords, and colors (hex, rgb, rgba, named). Cascade by specificity and source order, and inheritance for a small property set. Not supported: combinators (descendant, child, sibling), pseudo classes and pseudo elements, attribute selectors, at rules such as media and font face, shorthand expansion beyond the direct `margin`, `padding`, `border-width`, and `border-color` fallbacks, and units other than pixels.
