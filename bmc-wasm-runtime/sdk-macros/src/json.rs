// Copyright (C) 2026  Braiins Systems s.r.o.

//! Compile-time JSON template → `fmt!(...)` code generator.
//!
//! Parses a JSON-like token stream with `#(expr)` / `#s(expr)` interpolations
//! and emits a `bmc_wasm_sdk::fmt!()` call with all `{` / `}` pre-escaped.

use proc_macro2::{Delimiter, Literal, TokenStream, TokenTree};
use quote::quote;

/// Entry point: parse the full token stream and emit the resulting expression.
pub fn expand(input: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut pos = 0;
    let mut fmt_str = String::new();
    let mut args: Vec<TokenStream> = Vec::new();

    parse_value(&tokens, &mut pos, &mut fmt_str, &mut args);

    if pos < tokens.len() {
        let span = tokens[pos].span();
        return syn::Error::new(span, "unexpected tokens after JSON value").to_compile_error();
    }

    if args.is_empty() {
        // Static JSON — emit a string literal directly
        let lit = Literal::string(&fmt_str);
        quote! { String::from(#lit) }
    } else {
        quote! { bmc_wasm_sdk::fmt!(#fmt_str, #(#args),*) }
    }
}

// ── Recursive descent parser ────────────────────────────────────

fn parse_value(
    tokens: &[TokenTree],
    pos: &mut usize,
    fmt: &mut String,
    args: &mut Vec<TokenStream>,
) {
    let Some(tok) = tokens.get(*pos) else {
        panic!("json!: unexpected end of input");
    };
    match tok {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            parse_object(tokens, pos, fmt, args);
        }
        TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => {
            parse_array(tokens, pos, fmt, args);
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
                _ => panic!("json!: unexpected identifier `{s}`"),
            }
        }
        TokenTree::Punct(p) if p.as_char() == '#' => {
            parse_interpolation(tokens, pos, fmt, args);
        }
        // Negative number: `-` followed by literal
        TokenTree::Punct(p) if p.as_char() == '-' => {
            fmt.push('-');
            *pos += 1;
            if let Some(TokenTree::Literal(_)) = tokens.get(*pos) {
                parse_literal(tokens, pos, fmt);
            } else {
                panic!("json!: expected number after `-`");
            }
        }
        other => {
            panic!("json!: unexpected token: {other}");
        }
    }
}

fn parse_object(
    tokens: &[TokenTree],
    pos: &mut usize,
    fmt: &mut String,
    args: &mut Vec<TokenStream>,
) {
    let TokenTree::Group(g) = &tokens[*pos] else {
        unreachable!();
    };
    let inner: Vec<TokenTree> = g.stream().into_iter().collect();
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
                    panic!("json!: object key must be a string, got `{repr}`");
                }
                fmt.push_str(&repr);
                ipos += 1;
            }
            other => panic!("json!: expected string key, got `{other}`"),
        }

        // Colon
        match inner.get(ipos) {
            Some(TokenTree::Punct(p)) if p.as_char() == ':' => ipos += 1,
            other => panic!("json!: expected `:` after key, got {other:?}"),
        }

        fmt.push_str(": ");
        parse_value(&inner, &mut ipos, fmt, args);

        // Optional comma
        if let Some(TokenTree::Punct(p)) = inner.get(ipos) {
            if p.as_char() == ',' {
                ipos += 1;
            }
        }
    }
    fmt.push_str("}}");
}

fn parse_array(
    tokens: &[TokenTree],
    pos: &mut usize,
    fmt: &mut String,
    args: &mut Vec<TokenStream>,
) {
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
        parse_value(&inner, &mut ipos, fmt, args);

        // Optional comma
        if let Some(TokenTree::Punct(p)) = inner.get(ipos) {
            if p.as_char() == ',' {
                ipos += 1;
            }
        }
    }
    fmt.push(']');
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
) {
    // Skip the `#`
    *pos += 1;

    // Check for `s` (string interpolation) before the group
    let is_string = match tokens.get(*pos) {
        Some(TokenTree::Ident(id)) if id.to_string() == "s" => {
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
        other => panic!("json!: expected `(expr)` after `#`, got {other:?}"),
    };

    if is_string {
        // #s(expr) → "value" in JSON output — push real quotes into the format string
        fmt.push('"');
        fmt.push_str("{}");
        fmt.push('"');
    } else {
        // #(expr) → "{}" in format string
        fmt.push_str("{}");
    }
    args.push(group);
}
