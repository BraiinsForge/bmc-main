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

//! What the gRPC tests share: a handler request shaped like the dispatcher's,
//! an in-process client transport, and a reader of `BadRequest` details.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use bmc_grpc::google::rpc::BadRequest;
use bmc_grpc::google::rpc::bad_request::FieldViolation;
use bmc_intl::{Intl, Language, Translations};
use buffa::view::HasMessageView;
use buffa::{Message as _, MessageName as _};
use bytes::Bytes;
use connectrpc::client::{ClientConfig, ServiceTransport};
use connectrpc::{ConnectError, Protocol, RequestContext};
use std::collections::BTreeSet;
use std::path::Path;

use super::configuration_service::{first_day_of_week_unchosen, validate_sound_volume};
use super::language::offered_language;
use super::managed_by_boser;
use super::system::{
    failed_password_check, failed_password_presence, failed_password_set, invalid_timezone,
    wrong_current_password,
};
use crate::web::session::missing_session;

mod alarm;
mod boser_managed;
mod boser_upgrade;
mod language;
mod metadata;
mod network;

/// Binds `$request` to the `ServiceRequest` of `$message`, decoded from its own wire bytes
/// as the dispatcher hands it over; the bytes and the view live in the caller's scope.
macro_rules! wire_request {
    ($request:ident, $message:expr) => {
        let message = $message;
        let body = ::bytes::Bytes::from(::buffa::Message::encode_to_vec(&message));
        let view = $crate::web::grpc::tests::decode_view(&message, &body);
        let $request = ::connectrpc::ServiceRequest::from_parts(&view, &body);
    };
}
pub(crate) use wire_request;

/// The view of `body`, typed after `message`, which it encodes.
pub(crate) fn decode_view<'a, Req: HasMessageView>(
    _message: &Req,
    body: &'a Bytes,
) -> Req::View<'a> {
    Req::decode_view(body).expect("BUG: an encoded message decodes")
}

pub(crate) fn no_session() -> RequestContext {
    RequestContext::new(http::HeaderMap::new())
}

/// A client transport over `router`, no socket involved.
pub(crate) fn transport(router: axum::Router) -> ServiceTransport<axum::Router> {
    ServiceTransport::new(router)
}

/// A gRPC client configuration for [`transport`].
pub(crate) fn grpc() -> ClientConfig {
    ClientConfig::new("http://bmc".parse().expect("BUG: a literal URI parses"))
        .with_protocol(Protocol::Grpc)
}

/// A gRPC-Web client configuration for [`transport`], the protocol the browser speaks.
pub(crate) fn grpc_web() -> ClientConfig {
    ClientConfig::new("http://bmc".parse().expect("BUG: a literal URI parses"))
        .with_protocol(Protocol::GrpcWeb)
}

/// The violations in the `google.rpc.BadRequest` details of `error`.
pub(crate) fn field_violations(error: &ConnectError) -> Vec<FieldViolation> {
    error
        .details
        .iter()
        .filter(|detail| detail.type_url.ends_with(BadRequest::FULL_NAME))
        .flat_map(|detail| {
            let bytes = STANDARD_NO_PAD
                .decode(detail.value.as_deref().unwrap_or_default())
                .expect("BUG: a detail the server encoded decodes");
            BadRequest::decode_from_slice(&bytes)
                .expect("BUG: a BadRequest detail decodes")
                .field_violations
        })
        .collect()
}

/// The fields named by the `google.rpc.BadRequest` details of `error`.
pub(crate) fn violation_fields(error: &ConnectError) -> Vec<String> {
    field_violations(error)
        .into_iter()
        .map(|violation| violation.field)
        .collect()
}

fn pseudo_pack() -> Translations {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(bmc_intl_pseudo::PACK_ROOT);
    let (translations, report) = Translations::load(std::slice::from_ref(&root));
    assert!(report.is_clean(), "{report}");
    assert!(
        translations.languages().contains(&Language::EN_XA),
        "no en-XA pack under {}, is it in this crate's nix include?",
        root.display()
    );
    translations
}

/// An en-XA formatter that leaves the process's own translations as they are.
pub(crate) fn pseudo_locale() -> Intl {
    pseudo_pack().formatter(Language::EN_XA)
}

/// The samples of every name of an enum.
/// The exhaustive match in `samples` proves every name has an arm,
/// but not that the arm is non-empty or that its samples carry that name.
pub(crate) fn sampled<Name, Sample>(samples: impl Fn(Name) -> Vec<Sample>) -> Vec<Sample>
where
    Name: strum::IntoEnumIterator + Copy + PartialEq + std::fmt::Debug + for<'a> From<&'a Sample>,
    Sample: std::fmt::Debug,
{
    Name::iter()
        .flat_map(|name| {
            let listed = samples(name);
            assert!(!listed.is_empty(), "{name:?} has no sample");
            for sample in &listed {
                assert_eq!(
                    Name::from(sample),
                    name,
                    "{sample:?} is listed under {name:?} instead of its own name"
                );
            }
            listed
        })
        .collect()
}

/// Fails unless each refusal reads, in the en-XA pseudo-locale, as its English respelled:
/// a message the extraction missed stays plain English there, and one glued to a literal
/// comes out half respelled.
///
/// Values must hold no letters, as data is not respelled.
/// The refusals must differ in English, so an input landing in the wrong branch fails too.
pub(crate) fn assert_worded(refusals: impl Fn(&Intl) -> Vec<String>) {
    let translations = pseudo_pack();
    let english = refusals(&translations.formatter(Language::EN));
    assert!(!english.is_empty(), "BUG: a list with nothing to check");
    let distinct: BTreeSet<&String> = english.iter().collect();
    assert_eq!(
        distinct.len(),
        english.len(),
        "two inputs reached one refusal: {english:#?}"
    );

    let spoken = refusals(&translations.formatter(Language::EN_XA));
    assert_eq!(
        spoken.len(),
        english.len(),
        "en-XA and English gave a different number of refusals: {spoken:#?} vs {english:#?}"
    );
    for (message, spoken) in english.iter().zip(&spoken) {
        assert_eq!(*spoken, bmc_intl_pseudo::respell(message), "{message}");
    }
}

/// The reasons of exactly `N` violations.
pub(crate) fn reasons<const N: usize>(violations: impl Into<Vec<FieldViolation>>) -> [String; N] {
    let reasons: Vec<String> = violations
        .into()
        .into_iter()
        .map(|violation| violation.description)
        .collect();
    reasons
        .try_into()
        .unwrap_or_else(|found: Vec<String>| panic!("expected {N} reasons, got {found:#?}"))
}

/// The reason of the one violation.
pub(crate) fn the_reason(violations: impl Into<Vec<FieldViolation>>) -> String {
    let [reason] = reasons(violations);
    reason
}

/// The text of `error` itself, above any reasons it carries.
pub(crate) fn status_text(error: &ConnectError) -> String {
    error
        .message
        .clone()
        .expect("BUG: every status the device words has a message")
}

#[test]
fn the_access_refusals_are_worded() {
    assert_worded(|intl| {
        vec![
            status_text(&missing_session(intl)),
            status_text(&managed_by_boser(intl)),
        ]
    });
}

#[test]
fn the_refusals_inside_handlers_are_worded() {
    let (translations, _) = Translations::load(&[]);
    assert_worded(|intl| {
        let unavailable =
            offered_language(intl, &translations, "1").expect_err("BUG: no language is called 1");
        vec![
            the_reason(field_violations(&unavailable)),
            the_reason(field_violations(&invalid_timezone(intl))),
            the_reason(field_violations(&wrong_current_password(intl, "password"))),
            the_reason(field_violations(&first_day_of_week_unchosen(intl))),
            status_text(&failed_password_presence(intl)),
            status_text(&failed_password_check(intl)),
            status_text(&failed_password_set(intl)),
            status_text(&validate_sound_volume(intl, u32::MAX).expect_err("BUG: out of range")),
        ]
    });
}
