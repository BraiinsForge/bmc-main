// Copyright (C) 2025  Braiins Systems s.r.o.
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

use std::net::SocketAddr;
use std::time::Instant;

use axum::extract::ConnectInfo;
use connectrpc::interceptor::{
    Next, NextStream, PayloadStream, StreamRequest, StreamResponse, UnaryRequest, UnaryResponse,
};
use connectrpc::{ConnectError, Interceptor, RequestContext};
use tracing::info;

/// One `info!` line per call, with the outcome a unary call ends in;
/// a stream is logged as it opens, since its end never passes through here.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RpcLog;

#[connectrpc::async_trait]
impl Interceptor for RpcLog {
    async fn intercept_unary(
        &self,
        req: UnaryRequest,
        next: Next<'_>,
    ) -> Result<UnaryResponse, ConnectError> {
        let call = Call::start(&req.ctx);
        let response = next.run(req).await;
        call.log(response.as_ref().err());
        response
    }

    async fn intercept_streaming(
        &self,
        req: StreamRequest,
        inbound: PayloadStream,
        next: NextStream<'_>,
    ) -> Result<StreamResponse, ConnectError> {
        let call = Call::start(&req.ctx);
        let response = next.run(req, inbound).await;
        call.log(response.as_ref().err());
        response
    }
}

struct Call {
    client_ip: String,
    method: String,
    started: Instant,
}

/// Logs a call the access gate refused, which never reaches [`RpcLog`].
pub(crate) fn log_refusal(extensions: &http::Extensions, path: &str, refusal: &ConnectError) {
    info!(
        client_ip = %client_ip(extensions),
        method = %path,
        status = refusal.code.as_str(),
        grpc_code = refusal.code.grpc_code(),
        "gRPC request error"
    );
}

fn client_ip(extensions: &http::Extensions) -> String {
    extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map_or_else(|| String::from("-"), |ConnectInfo(addr)| addr.to_string())
}

impl Call {
    fn start(ctx: &RequestContext) -> Self {
        Self {
            client_ip: client_ip(ctx.extensions()),
            method: ctx.path().unwrap_or("-").to_owned(),
            started: Instant::now(),
        }
    }

    fn log(&self, error: Option<&ConnectError>) {
        let latency = format!("{:.6}", self.started.elapsed().as_secs_f64());
        match error {
            None => info!(
                client_ip = %self.client_ip,
                method = %self.method,
                status = "OK",
                latency,
                "gRPC request"
            ),
            Some(error) => info!(
                client_ip = %self.client_ip,
                method = %self.method,
                status = error.code.as_str(),
                grpc_code = error.code.grpc_code(),
                latency,
                "gRPC request error"
            ),
        }
    }
}
