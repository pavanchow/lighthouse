# Lighthouse

A from scratch browser rendering engine with zero external dependencies, written in pure Rust (edition 2021). Lighthouse takes HTML and CSS and runs them through the same pipeline a real browser uses, one inspectable stage at a time.

```text
HTML text   -> [html]   -> DOM tree
CSS text    -> [css]    -> Stylesheet
DOM + CSS   -> [style]  -> Styled tree (computed values)
Styled tree -> [layout] -> Layout tree (absolute rectangles)
Layout tree -> [paint]  -> Display list + raster
```

Live playground: https://pavanchow.github.io/lighthouse/

## What it is

Lighthouse is a teaching grade rendering engine that implements the real core of a browser: an HTML tokenizer and tree builder, a CSS parser with a proper cascade, style resolution with specificity and inheritance, a block and inline box model layout, and a paint stage that emits a display list and a headless raster. There is no networking, no JavaScript engine, and no GPU. There is just the part that turns markup and styles into positioned boxes.

## The gap it fills

Real engines (Blink, WebKit, Gecko) are millions of lines and effectively opaque. Toy examples usually stop at parsing. Lighthouse sits in between. It is small enough to read in an afternoon and complete enough to show every stage of the pipeline with real intermediate representations you can print and assert on.

That matters for two kinds of user:

- A person learning how browsers work gets a runnable, inspectable model. You can feed in a document and watch the DOM, the computed styles, and the exact box rectangles fall out.
- An AI agent or tool that needs to reason about layout gets a dependency free library with stable, inspectable data types at every stage. No headless Chrome, no opaque black box. You can call one stage, read its output, and feed it into the next.

## Quickstart

```sh
cargo run -- demo                       # render a built in sample, print every stage
cargo run -- page.html                  # render a file (uses its <style> blocks)
cargo run -- page.html --css sheet.css  # add a companion stylesheet
cargo test                              # run the correctness gates and unit tests
```

The CLI prints, in order, the parsed DOM, the computed styles, the layout box tree, and an ASCII raster of the paint stage.

## API

Every stage is a module with plain data types, so you can stop and inspect at any point.

```rust
use lighthouse::{html, css, style, layout, paint};
use lighthouse::layout::{Dimensions, Rect};

let dom = html::parse("<div class=box>Hello</div>");
let sheet = css::parse(".box { display: block; padding: 8px; background: #eee; }");
let styled = style::style_tree(&dom, &sheet);

let viewport = Dimensions {
    content: Rect { x: 0.0, y: 0.0, width: 800.0, height: 0.0 },
    ..Default::default()
};
let boxes = layout::layout_tree(&styled, viewport);
let display_list = paint::build_display_list(&boxes);

println!("{}", dom.pretty());
println!("{}", style::pretty(&styled));
println!("{}", layout::pretty(&boxes));
```

Key entry points:

- `html::parse(&str) -> dom::Node`
- `css::parse(&str) -> css::Stylesheet`
- `style::style_tree(&Node, &Stylesheet) -> StyledNode`
- `layout::layout_tree(&StyledNode, Dimensions) -> LayoutBox`
- `paint::build_display_list(&LayoutBox) -> DisplayList` and `paint::Canvas` for a raster

## The correctness gate

Lighthouse commits its claims as tests. Three gates back the three hardest stages, and each stage also has unit tests. Run them all with `cargo test`.

1. **HTML parse correctness** (`tests/html_parse.rs`). Test documents parse into an exact expected tree: tag names, nesting, attributes, and text, including void elements and implicit closing. A round trip test serializes the DOM back to HTML and reparses it, asserting the two trees are equal.
2. **CSS cascade and specificity** (`tests/css_cascade.rs`). For nodes matched by several rules, the winning declaration is the one with the highest specificity, then the latest source order. This is checked against hand specified expectations and against a reference specificity calculation.
3. **Layout invariants** (`tests/layout.rs`). Over randomly generated documents, every in flow block child stays inside its parent content box, block siblings stack without vertical overlap, and no box has a negative dimension. Golden tests pin down exact rectangles computed by hand. The random run is bounded for CI and is reproducible through `LIGHTHOUSE_FUZZ_OPS` and `LIGHTHOUSE_FUZZ_SEED`.

```sh
LIGHTHOUSE_FUZZ_OPS=5000 LIGHTHOUSE_FUZZ_SEED=1 cargo test fuzz_layout_invariants_hold
```

## Documented subset

Lighthouse implements a clear subset rather than the full web platform. See `DESIGN.md` for the exact HTML and CSS that are supported, the layout algorithm, and why each gate proves what it claims.

## License

MIT.
