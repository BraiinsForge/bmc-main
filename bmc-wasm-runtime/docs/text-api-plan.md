# Text API Plan

> **Status:** Draft - pending implementation

## Goals

- Word-wrap support for paragraph text
- Styled text (bold, italic, underline, strikethrough)
- Rich paragraphs with mixed styles per span
- Ergonomic SDK API with sensible defaults
- Changes to interface don't break user code (named fields via macros)

## TextStyle Struct

```rust
pub struct TextStyle {
    pub size: u32,           // default: 16 (pixels)
    pub color: u32,          // default: GRAY_10 (RGBA)
    pub max_width: u32,      // default: 0 (no wrap)
    pub weight: u16,         // default: 400 (normal), 700 = bold
    pub italic: bool,        // default: false
    pub underline: bool,     // default: false
    pub strikethrough: bool, // default: false
    pub line_height: f32,    // default: 1.4 (multiplier)
}
```

Construction via macro (like `props!`):

```rust
text_style!()                           // all defaults
text_style!(size: 24)                   // override size only
text_style!(size: 16, weight: 700)      // bold body text
text_style!(color: RED, underline: true)
```

## Simple Text

For single-style text blocks:

```rust
text("Hello world", text_style!(size: 24))
text("Wrapped paragraph...", text_style!(size: 14, max_width: 400))
```

## Rich Paragraphs (Mixed Styles)

For paragraphs with inline styling variations:

### SDK Types

```rust
pub struct Span {
    text: String,
    style: TextStyle,  // fully resolved (base + overrides merged)
}

// Auto-convert &str to Span (inherits base style, no overrides)
impl From<&str> for Span
```

### Span Helpers

```rust
fn bold(text: &str) -> SpanBuilder       // weight: 700
fn italic(text: &str) -> SpanBuilder     // italic: true
fn underline(text: &str) -> SpanBuilder  // underline: true
fn strike(text: &str) -> SpanBuilder     // strikethrough: true
fn styled(text: &str, overrides: TextStyle) -> SpanBuilder
```

### Usage

```rust
paragraph(
    x, y,
    text_style!(size: 16, max_width: 400),  // base style
    [
        "Click ",
        bold("Save"),
        " to ",
        styled("confirm", text_style!(color: GREEN)),
        " your changes.",
    ]
)
```

The SDK merges each span's overrides with the base style before serialization.

## FFI Design

Single host call with serialized data (avoids multiple FFI roundtrips):

```rust
fn host_draw_paragraph(
    x: i32,
    y: i32,
    base_style_ptr: u32,   // pointer to serialized TextStyle
    base_style_len: u32,
    spans_ptr: u32,        // pointer to serialized spans array
    spans_count: u32,
) -> u32;  // returns total height used (for layout)
```

### Serialization Format

**TextStyle (fixed size, 16 bytes):**

```
[size: u32][color: u32][max_width: u32][flags: u32]
                                        ^-- weight(12 bits), line_height(12 bits),
                                            italic(1), underline(1), strike(1), reserved(5)
```

**Spans array:**

```
[span 0: text_ptr: u32, text_len: u32, style: 16 bytes]
[span 1: ...]
...
```

Each span has fully resolved style (base + overrides already merged by SDK).

## Host Implementation

1. Deserialize base style and spans from WASM memory
2. Build `Vec<(&str, cosmic_text::Attrs)>` from spans
3. Call `buffer.set_rich_text(font_system, spans, Shaping::Advanced)`
4. Render glyphs via existing draw callback
5. If underline/strikethrough flags set, draw decoration lines using font metrics
6. Return total height consumed

## Migration

Since this is pre-release with no external users:

- Replace existing `host_draw_text` with new signature
- Update SDK's `text()` function to use new API
- Existing `text("...", size, props)` calls migrate to `text("...", text_style!(size: N, color: C))`

## Open Questions

- [ ] Should `paragraph()` return height for layout purposes?
- [ ] Do we need text alignment (left/center/right)?
- [ ] Letter-spacing / word-spacing needed?
- [ ] Should spans support size changes or just weight/style/color/decoration?
