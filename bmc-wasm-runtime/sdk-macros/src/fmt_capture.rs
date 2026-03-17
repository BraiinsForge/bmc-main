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

/// Result of rewriting a format string.
struct Rewritten {
    /// The new format string with captures replaced by `{}` or `{:spec}`.
    format_string: String,
    /// Captured identifier expressions (e.g. `year`, `self.field`).
    captured: Vec<String>,
}

/// Walk the format string and rewrite `{ident}` / `{ident:spec}` into
/// `{}` / `{:spec}`, collecting captured identifiers.
fn rewrite_format_string(raw: &str) -> Rewritten {
    let mut out = String::with_capacity(raw.len());
    let mut captured = Vec::new();
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
                // `{}` — plain positional arg
                out.push_str("{}");
                i += 1;
                continue;
            }

            if next == ':' || next == '?' || next == '#' {
                // `{:spec}`, `{?}`, `{#...}` — positional with spec, emit as-is
                // Collect everything up to `}`
                out.push('{');
                while i < len && chars[i] != '}' {
                    out.push(chars[i]);
                    i += 1;
                }
                if i < len {
                    out.push('}');
                    i += 1;
                }
                continue;
            }

            // Check if this is a digit (positional index like `{0}`, `{1:x}`)
            if next.is_ascii_digit() {
                // Positional index — emit as-is
                out.push('{');
                while i < len && chars[i] != '}' {
                    out.push(chars[i]);
                    i += 1;
                }
                if i < len {
                    out.push('}');
                    i += 1;
                }
                continue;
            }

            // Check if this starts an identifier (capture candidate)
            if next.is_ascii_alphabetic() || next == '_' {
                let ident_start = i;
                // Collect identifier: alphanumeric, underscore, dots (for paths)
                while i < len
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
                {
                    i += 1;
                }
                let ident = &raw[ident_start..char_to_byte(&chars, i, raw)];

                if i < len && chars[i] == '}' {
                    // `{ident}` — capture without spec
                    out.push_str("{}");
                    captured.push(ident.to_string());
                    i += 1;
                    continue;
                }

                if i < len && chars[i] == ':' {
                    // `{ident:spec}` — capture with format spec
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
                    captured.push(ident.to_string());
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
        captured,
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
    let positional = &parsed.positional_args;

    // Use the span from the format string literal so that captured identifiers
    // resolve at the call site (important when the proc macro is invoked through
    // a declarative macro wrapper like `fmt!`).
    let call_span = parsed.format_str.span();

    // Convert captured identifiers to token streams with correct spans.
    // Supports dotted paths like `self.field` or `foo.bar`.
    let captured_tokens: Vec<TokenStream> = rewritten
        .captured
        .iter()
        .map(|ident_str| ident_path_tokens(ident_str, call_span))
        .collect();

    let ufmt_path = &parsed.ufmt_path;

    quote_spanned! { call_span => {
        let mut __fmt_buf = ::std::string::String::new();
        _ = #ufmt_path::uwrite!(__fmt_buf, #new_fmt #(, #positional)* #(, #captured_tokens)*);
        __fmt_buf
    }}
}
