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
use buffa::view::HasMessageView;
use buffa::{Message as _, MessageName as _};
use bytes::Bytes;
use connectrpc::client::{ClientConfig, ServiceTransport};
use connectrpc::{ConnectError, Protocol, RequestContext};

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

/// The fields named by the `google.rpc.BadRequest` details of `error`.
pub(crate) fn violation_fields(error: &ConnectError) -> Vec<String> {
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
        .map(|violation| violation.field)
        .collect()
}
