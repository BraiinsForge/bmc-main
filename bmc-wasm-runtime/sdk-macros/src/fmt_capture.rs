// Copyright (C) 2026  Braiins Systems s.r.o.

//! Format-string capture rewriter for `fmt!`.
//!
//! Parses a format string for `{ident}` and `{ident:spec}` patterns and
//! rewrites them into positional `{}` / `{:spec}` placeholders, appending
//! the captured identifiers as trailing arguments to `ufmt::uwrite!`.

use proc_macro2::{Ident, Punct, Spacing, Span, TokenStream, TokenTree};
use quote::quote_spanned;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, LitStr, Path, Token};

/// Parsed input: `@ufmt_path = <path>; <format_str> [, args...]`
struct FmtInput {
    ufmt_path: Path,
    format_str: LitStr,
    positional_args: Vec<Expr>,
}

impl Parse for FmtInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        // Parse @ufmt_path = <path>;
        let _at: Token![@] = input.parse()?;
        let label: Ident = input.parse()?;
        if label != "ufmt_path" {
            return Err(syn::Error::new(label.span(), "expected `ufmt_path`"));
        }
        let _eq: Token![=] = input.parse()?;
        let ufmt_path: Path = input.parse()?;
        let _semi: Token![;] = input.parse()?;

        let format_str: LitStr = input.parse()?;
        let mut positional_args = Vec::new();
        while input.peek(Token![,]) {
            let _comma: Token![,] = input.parse()?;
            // Allow trailing comma
            if input.is_empty() {
                break;
            }
            positional_args.push(input.parse()?);
        }
        Ok(Self {
            ufmt_path,
            format_str,
            positional_args,
        })
    }
}

/// One argument slot in a rewritten format string, in the order the
/// `uwrite!` call must receive them.
enum ArgSlot {
    /// `{}` / `{:spec}` — fill from the macro's positional args at this 0-based index.
    Positional(usize),
    /// `{ident}` / `{ident:spec}` — captured identifier path (dotted paths supported).
    Captured(String),
}

/// Result of rewriting a format string.
struct Rewritten {
    /// The new format string with captures replaced by `{}` or `{:spec}`.
    format_string: String,
    /// Argument slots in the order `uwrite!` must consume them.
    args: Vec<ArgSlot>,
}

/// Walk the format string. Each `{}` / `{:spec}` becomes a `Positional`
/// slot (filled from the macro caller's positional args in order); each
/// `{ident}` / `{ident:spec}` becomes a `Captured` slot. Slots are recorded
/// in format-string order so `uwrite!` consumes them correctly when
/// positional and captured placeholders interleave.
fn rewrite_format_string(raw: &str) -> Rewritten {
    let mut out = String::with_capacity(raw.len());
    let mut args: Vec<ArgSlot> = Vec::new();
    let mut positional_seen: usize = 0;
    let chars: Vec<char> = raw.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '{' {
            if i + 1 < len && chars[i + 1] == '{' {
                // Escaped brace `{{`
                out.push_str("{{");
                i += 2;
                continue;
            }

            // We're inside a `{...}` placeholder
            let start = i;
            i += 1; // skip `{`

            if i >= len {
                // Unterminated — just emit as-is, let ufmt error
                out.push('{');
                continue;
            }

            // Check what follows the `{`
            let next = chars[i];

            if next == '}' {
                // `{}` — anonymous positional, fill from macro positional args in order.
                out.push_str("{}");
                args.push(ArgSlot::Positional(positional_seen));
                positional_seen += 1;
                i += 1;
                continue;
            }

            if next == ':' {
                // `{:spec}` — anonymous positional with spec.
                out.push('{');
                while i < len && chars[i] != '}' {
                    out.push(chars[i]);
                    i += 1;
                }
                if i < len {
                    out.push('}');
                    i += 1;
                }
                args.push(ArgSlot::Positional(positional_seen));
                positional_seen += 1;
                continue;
            }

            // Check if this starts an identifier (capture candidate)
            if next.is_ascii_alphabetic() || next == '_' {
                let ident_start = char_to_byte(&chars, i, raw);
                // Collect identifier: alphanumeric, underscore, dots (for paths)
                while i < len
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
                {
                    i += 1;
                }
                let ident = raw[ident_start..char_to_byte(&chars, i, raw)].to_string();

                if i < len && chars[i] == '}' {
                    // `{ident}` — capture without spec.
                    out.push_str("{}");
                    args.push(ArgSlot::Captured(ident));
                    i += 1;
                    continue;
                }

                if i < len && chars[i] == ':' {
                    // `{ident:spec}` — capture with format spec.
                    i += 1; // skip `:`
                    let spec_start = i;
                    while i < len && chars[i] != '}' {
                        i += 1;
                    }
                    let spec =
                        &raw[char_to_byte(&chars, spec_start, raw)..char_to_byte(&chars, i, raw)];
                    out.push_str("{:");
                    out.push_str(spec);
                    out.push('}');
                    args.push(ArgSlot::Captured(ident));
                    if i < len {
                        i += 1; // skip `}`
                    }
                    continue;
                }

                // Not a valid capture — emit original text as-is
                out.push_str(&raw[char_to_byte(&chars, start, raw)..char_to_byte(&chars, i, raw)]);
                continue;
            }

            // Unknown content after `{` — emit as-is
            out.push('{');
            // Don't advance i, let the outer loop handle `chars[i]`
        } else if chars[i] == '}' && i + 1 < len && chars[i + 1] == '}' {
            // Escaped brace `}}`
            out.push_str("}}");
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }

    Rewritten {
        format_string: out,
        args,
    }
}

/// Convert a char index to a byte offset in the original string.
/// Handles multi-byte chars correctly.
fn char_to_byte(chars: &[char], char_idx: usize, raw: &str) -> usize {
    chars[..char_idx]
        .iter()
        .map(|c| c.len_utf8())
        .sum::<usize>()
        .min(raw.len())
}

/// Build a token stream for a (possibly dotted) identifier path with the given span.
/// E.g. `"foo.bar.baz"` → tokens for `foo.bar.baz`.
fn ident_path_tokens(path: &str, span: Span) -> TokenStream {
    let mut tokens = TokenStream::new();
    for (i, segment) in path.split('.').enumerate() {
        if i > 0 {
            let dot = Punct::new('.', Spacing::Alone);
            tokens.extend(std::iter::once(TokenTree::Punct(dot)));
        }
        let ident = Ident::new(segment, span);
        tokens.extend(std::iter::once(TokenTree::Ident(ident)));
    }
    tokens
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed: FmtInput = match syn::parse2(input) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error(),
    };

    let raw = parsed.format_str.value();
    let rewritten = rewrite_format_string(&raw);

    let new_fmt = LitStr::new(&rewritten.format_string, parsed.format_str.span());

    // Use the span from the format string literal so that captured identifiers
    // resolve at the call site (important when the proc macro is invoked through
    // a declarative macro wrapper like `fmt!`).
    let call_span = parsed.format_str.span();

    // Build the arg list in format-string order: each `{}`/`{:spec}`
    // slot points back to the macro caller's positional arg at that index,
    // and each `{ident}` slot expands to the captured identifier path.
    let arg_tokens: Vec<TokenStream> = rewritten
        .args
        .iter()
        .map(|slot| match slot {
            ArgSlot::Positional(idx) => {
                let expr = &parsed.positional_args[*idx];
                quote_spanned!(call_span => #expr)
            }
            ArgSlot::Captured(ident_str) => ident_path_tokens(ident_str, call_span),
        })
        .collect();

    let ufmt_path = &parsed.ufmt_path;

    quote_spanned! { call_span => {
        let mut __fmt_buf = ::std::string::String::new();
        _ = #ufmt_path::uwrite!(__fmt_buf, #new_fmt #(, #arg_tokens)*);
        __fmt_buf
    }}
}

#[cfg(test)]
mod tests {
    use super::{ArgSlot, rewrite_format_string};

    fn rewrite(raw: &str) -> (String, Vec<ArgSlot>) {
        let r = rewrite_format_string(raw);
        (r.format_string, r.args)
    }

    fn pos(idx: usize) -> ArgSlot {
        ArgSlot::Positional(idx)
    }

    fn cap(s: &str) -> ArgSlot {
        ArgSlot::Captured(s.into())
    }

    impl PartialEq for ArgSlot {
        fn eq(&self, other: &Self) -> bool {
            match (self, other) {
                (Self::Positional(a), Self::Positional(b)) => a == b,
                (Self::Captured(a), Self::Captured(b)) => a == b,
                _ => false,
            }
        }
    }

    impl core::fmt::Debug for ArgSlot {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::Positional(idx) => write!(f, "Positional({idx})"),
                Self::Captured(s) => write!(f, "Captured({s:?})"),
            }
        }
    }

    #[test]
    fn positional_only_passes_through_in_order() {
        let (s, args) = rewrite("{} = {}");
        assert_eq!(s, "{} = {}");
        assert_eq!(args, vec![pos(0), pos(1)]);
    }

    #[test]
    fn captures_simple_ident() {
        let (s, args) = rewrite("{year}-{month}");
        assert_eq!(s, "{}-{}");
        assert_eq!(args, vec![cap("year"), cap("month")]);
    }

    #[test]
    fn captures_with_format_spec() {
        let (s, args) = rewrite("{val:x}");
        assert_eq!(s, "{:x}");
        assert_eq!(args, vec![cap("val")]);
    }

    #[test]
    fn captures_dotted_path() {
        let (s, args) = rewrite("{self.field}");
        assert_eq!(s, "{}");
        assert_eq!(args, vec![cap("self.field")]);
    }

    #[test]
    fn escaped_braces_preserved() {
        let (s, args) = rewrite("{{literal}} {x}");
        assert_eq!(s, "{{literal}} {}");
        assert_eq!(args, vec![cap("x")]);
    }

    #[test]
    fn capture_after_multibyte_char() {
        // Regression: the parser used a char index where a byte index
        // was required, slicing into the middle of a multi-byte char
        // before the capture and producing a garbage identifier.
        let (s, args) = rewrite("{} — {desc}");
        assert_eq!(s, "{} — {}");
        assert_eq!(args, vec![pos(0), cap("desc")]);
    }

    #[test]
    fn capture_after_multiple_multibyte_chars() {
        let (s, args) = rewrite("Today \u{2022} {weekday}, {month} {}");
        assert_eq!(s, "Today \u{2022} {}, {} {}");
        assert_eq!(args, vec![cap("weekday"), cap("month"), pos(0)]);
    }

    #[test]
    fn capture_before_positional_keeps_format_order() {
        // Regression: `fmt!("{ident} {}", arg)` used to emit
        // `uwrite!(buf, "{} {}", arg, ident)`, which printed "arg ident"
        // instead of the intended "ident arg". The args list now follows
        // format-string order, so the captured slot is consumed first.
        let (s, args) = rewrite("{month_name} {}");
        assert_eq!(s, "{} {}");
        assert_eq!(args, vec![cap("month_name"), pos(0)]);
    }
}
