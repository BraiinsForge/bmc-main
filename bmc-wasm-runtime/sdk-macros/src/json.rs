// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! Compile-time JSON template → `fmt!(...)` code generator.
//!
//! Parses a JSON-like token stream with `#(expr)` / `#s(expr)` interpolations
//! and emits a `bmc_wasm_sdk::fmt!()` call with all `{` / `}` pre-escaped.

use proc_macro2::{Delimiter, Literal, Span, TokenStream, TokenTree};
use quote::quote;

/// Entry point: parse the full token stream and emit the resulting expression.
pub fn expand(input: TokenStream) -> TokenStream {
    expand_impl(input).unwrap_or_else(syn::Error::into_compile_error)
}

fn expand_impl(input: TokenStream) -> syn::Result<TokenStream> {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut pos = 0;
    let mut fmt_str = String::new();
    let mut args: Vec<TokenStream> = Vec::new();

    parse_value(&tokens, &mut pos, &mut fmt_str, &mut args)?;

    if pos < tokens.len() {
        return Err(syn::Error::new(
            tokens[pos].span(),
            "unexpected tokens after JSON value",
        ));
    }

    Ok(if args.is_empty() {
        // Static JSON — emit a string literal directly
        let lit = Literal::string(&fmt_str);
        quote! { String::from(#lit) }
    } else {
        quote! { bmc_wasm_sdk::fmt!(#fmt_str, #(#args),*) }
    })
}

// ── Recursive descent parser ────────────────────────────────────

fn parse_value(
    tokens: &[TokenTree],
    pos: &mut usize,
    fmt: &mut String,
    args: &mut Vec<TokenStream>,
) -> syn::Result<()> {
    let Some(tok) = tokens.get(*pos) else {
        return Err(syn::Error::new(
            tokens.last().map_or_else(Span::call_site, TokenTree::span),
            "json!: unexpected end of input",
        ));
    };
    match tok {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            parse_object(tokens, pos, fmt, args)?;
        }
        TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => {
            parse_array(tokens, pos, fmt, args)?;
        }
        TokenTree::Literal(_) => {
            parse_literal(tokens, pos, fmt);
        }
        TokenTree::Ident(id) => {
            let s = id.to_string();
            match s.as_str() {
                "true" | "false" | "null" => {
                    fmt.push_str(&s);
                    *pos += 1;
                }
                _ => {
                    return Err(syn::Error::new(
                        id.span(),
                        format!("json!: unexpected identifier `{s}`"),
                    ));
                }
            }
        }
        TokenTree::Punct(p) if p.as_char() == '#' => {
            parse_interpolation(tokens, pos, fmt, args)?;
        }
        // Negative number: `-` followed by literal
        TokenTree::Punct(p) if p.as_char() == '-' => {
            fmt.push('-');
            *pos += 1;
            if let Some(TokenTree::Literal(_)) = tokens.get(*pos) {
                parse_literal(tokens, pos, fmt);
            } else {
                return Err(syn::Error::new(
                    tokens.get(*pos).map_or_else(|| p.span(), TokenTree::span),
                    "json!: expected number after `-`",
                ));
            }
        }
        other => {
            return Err(syn::Error::new(
                other.span(),
                format!("json!: unexpected token: {other}"),
            ));
        }
    }
    Ok(())
}

fn parse_object(
    tokens: &[TokenTree],
    pos: &mut usize,
    fmt: &mut String,
    args: &mut Vec<TokenStream>,
) -> syn::Result<()> {
    let TokenTree::Group(g) = &tokens[*pos] else {
        unreachable!();
    };
    let inner: Vec<TokenTree> = g.stream().into_iter().collect();
    let group_span = g.span();
    *pos += 1;

    fmt.push_str("{{");
    let mut ipos = 0;
    let mut first = true;
    while ipos < inner.len() {
        if !first {
            fmt.push_str(", ");
        }
        first = false;

        // Key must be a string literal
        match &inner[ipos] {
            TokenTree::Literal(lit) => {
                let repr = lit.to_string();
                if !repr.starts_with('"') {
                    return Err(syn::Error::new(
                        lit.span(),
                        format!("json!: object key must be a string, got `{repr}`"),
                    ));
                }
                fmt.push_str(&repr);
                ipos += 1;
            }
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    format!("json!: expected string key, got `{other}`"),
                ));
            }
        }

        // Colon
        match inner.get(ipos) {
            Some(TokenTree::Punct(p)) if p.as_char() == ':' => ipos += 1,
            Some(other) => {
                return Err(syn::Error::new(
                    other.span(),
                    format!("json!: expected `:` after key, got `{other}`"),
                ));
            }
            None => {
                return Err(syn::Error::new(
                    group_span,
                    "json!: expected `:` after key, got end of object",
                ));
            }
        }

        fmt.push_str(": ");
        parse_value(&inner, &mut ipos, fmt, args)?;

        // Optional comma
        if let Some(TokenTree::Punct(p)) = inner.get(ipos)
            && p.as_char() == ','
        {
            ipos += 1;
        }
    }
    fmt.push_str("}}");
    Ok(())
}

fn parse_array(
    tokens: &[TokenTree],
    pos: &mut usize,
    fmt: &mut String,
    args: &mut Vec<TokenStream>,
) -> syn::Result<()> {
    let TokenTree::Group(g) = &tokens[*pos] else {
        unreachable!();
    };
    let inner: Vec<TokenTree> = g.stream().into_iter().collect();
    *pos += 1;

    fmt.push('[');
    let mut ipos = 0;
    let mut first = true;
    while ipos < inner.len() {
        if !first {
            fmt.push_str(", ");
        }
        first = false;
        parse_value(&inner, &mut ipos, fmt, args)?;

        // Optional comma
        if let Some(TokenTree::Punct(p)) = inner.get(ipos)
            && p.as_char() == ','
        {
            ipos += 1;
        }
    }
    fmt.push(']');
    Ok(())
}

fn parse_literal(tokens: &[TokenTree], pos: &mut usize, fmt: &mut String) {
    let TokenTree::Literal(lit) = &tokens[*pos] else {
        unreachable!();
    };
    // String literals include their quotes, numbers are bare — both go as-is.
    fmt.push_str(&lit.to_string());
    *pos += 1;
}

fn parse_interpolation(
    tokens: &[TokenTree],
    pos: &mut usize,
    fmt: &mut String,
    args: &mut Vec<TokenStream>,
) -> syn::Result<()> {
    // Skip the `#`
    let hash_span = tokens[*pos].span();
    *pos += 1;

    // Check for `s` (string interpolation) before the group
    let is_string = match tokens.get(*pos) {
        Some(TokenTree::Ident(id)) if *id == "s" => {
            *pos += 1;
            true
        }
        _ => false,
    };

    // Expect a parenthesized group
    let group = match tokens.get(*pos) {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            *pos += 1;
            g.stream()
        }
        Some(other) => {
            return Err(syn::Error::new(
                other.span(),
                format!("json!: expected `(expr)` after `#`, got `{other}`"),
            ));
        }
        None => {
            return Err(syn::Error::new(
                hash_span,
                "json!: expected `(expr)` after `#`, got end of input",
            ));
        }
    };

    if is_string {
        // #s(expr) → "value" in JSON output. Wrap the expression in
        // `JsonStr` so its `uDisplay` impl escapes `"`, `\`, and ASCII
        // control characters per RFC 8259 — without this, an interpolated
        // value containing `"` would terminate the surrounding string
        // literal and corrupt or inject into the output JSON.
        fmt.push('"');
        fmt.push_str("{}");
        fmt.push('"');
        args.push(quote! { bmc_wasm_sdk::JsonStr(#group) });
    } else {
        // #(expr) → "{}" in format string
        fmt.push_str("{}");
        args.push(group);
    }
    Ok(())
}
