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

//! `#[wasm_test]`: a libtest test natively, a pair of exports for the wasm test runner on wasm32.

use bmc_wasm_protocol::test_descriptor::{META_EXPORT_PREFIX, RUN_EXPORT_PREFIX, parse_fuel};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Attribute, Expr, ExprLit, ItemFn, Lit, LitStr, Meta, ReturnType, parse_quote};

/// The libtest attributes the macro takes off the function.
#[derive(Debug, Default, PartialEq, Eq)]
struct TestAttributes {
    libtest: Vec<Attribute>,
    ignored: bool,
    /// `Some` when the test must panic; the panic message must contain it.
    should_panic: Option<String>,
}

pub(crate) fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    match try_expand(attr, item) {
        Ok(expanded) => expanded,
        Err(error) => error.to_compile_error(),
    }
}

fn try_expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let fuel = fuel(attr)?.map_or_else(
        || quote!(::core::option::Option::None),
        |units| quote!(::core::option::Option::Some(#units)),
    );
    let mut function: ItemFn = syn::parse2(item)?;
    check_signature(&function)?;
    let attributes = take_test_attributes(&mut function.attrs)?;

    let name = &function.sig.ident;
    let libtest = &attributes.libtest;
    let flags = flags(&attributes);
    let expected = attributes.should_panic.unwrap_or_default();

    Ok(quote! {
        #[cfg(not(target_arch = "wasm32"))]
        #[test]
        #(#libtest)*
        #function

        #[cfg(all(test, target_arch = "wasm32"))]
        #function

        #[cfg(all(test, target_arch = "wasm32"))]
        const _: () = {
            static __BMC_RECORD: ::bmc_wasm_sdk::__test_support::TestRecord =
                ::bmc_wasm_sdk::__test_support::TestRecord::new(
                    ::core::concat!(::core::module_path!(), "::", ::core::stringify!(#name)),
                    #flags,
                    #expected,
                    #fuel,
                );

            #[unsafe(export_name = ::core::concat!(
                #META_EXPORT_PREFIX, ::core::module_path!(), "::", ::core::stringify!(#name)
            ))]
            extern "C" fn __bmc_locate_record() -> u64 {
                __BMC_RECORD.locate()
            }

            #[unsafe(export_name = ::core::concat!(
                #RUN_EXPORT_PREFIX, ::core::module_path!(), "::", ::core::stringify!(#name)
            ))]
            extern "C" fn __bmc_run() {
                ::bmc_wasm_sdk::__test_support::run(#name);
            }
        };
    })
}

/// The budget `#[wasm_test(fuel = …, reason = "…")]` asks for, `None` to take the runner's.
fn fuel(attr: TokenStream) -> syn::Result<Option<u64>> {
    let mut fuel: Option<(String, Span)> = None;
    let mut reason: Option<LitStr> = None;
    let parser = syn::meta::parser(|argument| {
        if argument.path.is_ident("fuel") {
            fuel = Some(match argument.value()?.parse()? {
                Lit::Int(units) => (units.base10_digits().to_owned(), units.span()),
                Lit::Float(units) => (units.base10_digits().to_owned(), units.span()),
                other => return Err(syn::Error::new_spanned(other, FUEL_UNITS)),
            });
        } else if argument.path.is_ident("reason") {
            reason = Some(argument.value()?.parse()?);
        } else {
            return Err(argument.error(
                "`#[wasm_test]` takes only `fuel = <units>, reason = \"…\"`; \
                 write `#[should_panic]` and `#[ignore]` after it",
            ));
        }
        Ok(())
    });
    syn::parse::Parser::parse2(parser, attr)?;
    match (fuel, reason) {
        (None, None) => Ok(None),
        (Some((_, span)), None) => Err(syn::Error::new(
            span,
            "a fuel budget needs a `reason = \"…\"` saying why the test is heavy",
        )),
        (None, Some(reason)) => Err(syn::Error::new_spanned(
            reason,
            "`reason` explains a `fuel` budget, and there is none",
        )),
        (Some(_), Some(reason)) if reason.value().trim().is_empty() => {
            Err(syn::Error::new_spanned(reason, "say why the test is heavy"))
        }
        (Some((digits, span)), Some(_)) => match parse_fuel(&digits) {
            Some(0) => Err(syn::Error::new(span, "a test needs some fuel to run")),
            Some(fuel) => Ok(Some(fuel)),
            None => Err(syn::Error::new(span, FUEL_UNITS)),
        },
    }
}

const FUEL_UNITS: &str = "fuel is a whole number of units, such as `250_000_000` or `250e6`";

fn check_signature(function: &ItemFn) -> syn::Result<()> {
    let signature = &function.sig;
    if signature.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            signature.asyncness,
            "a wasm test cannot be async",
        ));
    }
    if !signature.inputs.is_empty() || !signature.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &signature.inputs,
            "a wasm test takes no arguments and no generics",
        ));
    }
    if let ReturnType::Type(_, returned) = &signature.output {
        return Err(syn::Error::new_spanned(
            returned,
            "a wasm test returns `()`; report a failure by panicking",
        ));
    }
    Ok(())
}

/// Take `#[should_panic]` and `#[ignore]` off `attrs`, leaving every other attribute in place.
fn take_test_attributes(attrs: &mut Vec<Attribute>) -> syn::Result<TestAttributes> {
    let mut taken = TestAttributes::default();
    let mut kept = Vec::with_capacity(attrs.len());
    for attribute in attrs.drain(..) {
        if attribute.path().is_ident("ignore") {
            taken.ignored = true;
        } else if attribute.path().is_ident("should_panic") {
            taken.should_panic = Some(expected_message(&attribute)?);
        } else {
            kept.push(attribute);
            continue;
        }
        taken.libtest.push(attribute);
    }
    *attrs = kept;
    Ok(taken)
}

/// The `expected` of `#[should_panic]`, `#[should_panic = "…"]` or `#[should_panic(expected = "…")]`.
fn expected_message(attribute: &Attribute) -> syn::Result<String> {
    match &attribute.meta {
        Meta::Path(_) => Ok(String::new()),
        Meta::NameValue(name_value) => string_literal(&name_value.value),
        Meta::List(_) => {
            let mut expected = None;
            attribute.parse_nested_meta(|nested| {
                if nested.path.is_ident("expected") {
                    expected = Some(nested.value()?.parse::<LitStr>()?.value());
                    Ok(())
                } else {
                    Err(nested.error("`should_panic` takes only `expected = \"…\"`"))
                }
            })?;
            Ok(expected.unwrap_or_default())
        }
    }
}

fn string_literal(value: &Expr) -> syn::Result<String> {
    match value {
        Expr::Lit(ExprLit {
            lit: Lit::Str(text),
            ..
        }) => Ok(text.value()),
        other => Err(syn::Error::new_spanned(other, "expected a string literal")),
    }
}

fn flags(attributes: &TestAttributes) -> TokenStream {
    let mut flags: Vec<syn::Path> = Vec::new();
    if attributes.ignored {
        flags.push(parse_quote!(::bmc_wasm_sdk::__test_support::IGNORED));
    }
    if attributes.should_panic.is_some() {
        flags.push(parse_quote!(::bmc_wasm_sdk::__test_support::SHOULD_PANIC));
    }
    if flags.is_empty() {
        quote!(0)
    } else {
        quote!(#(#flags)|*)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taken(attrs: &[Attribute]) -> syn::Result<TestAttributes> {
        take_test_attributes(&mut attrs.to_vec())
    }

    #[test]
    fn should_panic_carries_its_expected_message() {
        let attributes = taken(&[parse_quote!(#[should_panic(expected = "month 0")])])
            .expect("BUG: a well-formed `should_panic` parses");

        assert_eq!(attributes.should_panic.as_deref(), Some("month 0"));
    }

    #[test]
    fn a_bare_should_panic_expects_any_message() {
        let attributes =
            taken(&[parse_quote!(#[should_panic])]).expect("BUG: a bare `should_panic` parses");

        assert_eq!(attributes.should_panic.as_deref(), Some(""));
    }

    #[test]
    fn ignore_is_taken_and_other_attributes_stay() {
        let mut attrs: Vec<Attribute> = vec![
            parse_quote!(#[ignore = "slow"]),
            parse_quote!(#[expect(clippy::unwrap_used)]),
        ];
        let attributes =
            take_test_attributes(&mut attrs).expect("BUG: `ignore` with a reason parses");

        assert_eq!(
            (attributes.ignored, attrs.len()),
            (true, 1),
            "only `ignore` leaves the function"
        );
    }

    #[test]
    fn an_unknown_should_panic_argument_is_refused() {
        assert!(taken(&[parse_quote!(#[should_panic(reason = "x")])]).is_err());
    }

    #[test]
    fn a_fuel_argument_sets_the_test_budget() {
        assert_eq!(
            fuel(quote!(fuel = 300_000_000, reason = "a large fixture"))
                .expect("BUG: a fuel budget with its reason parses"),
            Some(300_000_000)
        );
    }

    #[test]
    fn a_fuel_budget_takes_exponent_form() {
        assert_eq!(
            fuel(quote!(fuel = 2.5e8, reason = "a large fixture"))
                .expect("BUG: a fuel budget in exponent form parses"),
            Some(250_000_000)
        );
    }

    #[test]
    fn a_fuel_budget_is_a_whole_number_of_units() {
        assert!(fuel(quote!(fuel = 2.5, reason = "a large fixture")).is_err());
        assert!(fuel(quote!(fuel = "lots", reason = "a large fixture")).is_err());
    }

    #[test]
    fn a_fuel_budget_needs_a_reason() {
        assert!(fuel(quote!(fuel = 300_000_000)).is_err());
        assert!(fuel(quote!(fuel = 300_000_000, reason = " ")).is_err());
    }

    #[test]
    fn a_reason_without_a_budget_is_refused() {
        assert!(fuel(quote!(reason = "a large fixture")).is_err());
    }

    #[test]
    fn no_argument_leaves_the_runner_budget() {
        assert_eq!(
            fuel(TokenStream::new()).expect("BUG: no argument parses"),
            None
        );
    }

    #[test]
    fn zero_fuel_and_unknown_arguments_are_refused() {
        assert!(fuel(quote!(fuel = 0, reason = "none at all")).is_err());
        assert!(fuel(quote!(timeout = 5)).is_err());
    }

    #[test]
    fn a_test_that_returns_a_value_is_refused() {
        let item: TokenStream = quote! { fn returns() -> u32 { 1 } };

        assert!(try_expand(TokenStream::new(), item).is_err());
    }
}
