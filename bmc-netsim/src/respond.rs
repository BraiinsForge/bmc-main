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

//! HTTP responder: serve a resource's endpoints on its own TCP port.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use axum::extract::Query;
use axum::http::{HeaderMap, header};
use axum::response::IntoResponse;
use axum::{Json, Router, routing};

use crate::blueprint::{EndpointSpec, RequestCtx, Response, ResponseData, ResponseSpec};
use crate::cache::Cache;
use crate::render;

/// Build the router.
/// `Render` fills its template per request, from `seed` and `start`;
/// `Static` answers as it stands; `Computed` decides the whole response.
pub fn build_router(
    endpoints: Vec<EndpointSpec>,
    seed: u64,
    start: Instant,
    cache: &Arc<Cache>,
) -> Result<Router> {
    let mut router = Router::new();
    for endpoint in endpoints {
        let spec = endpoint.response.clone();
        let cache = Arc::clone(cache);
        let handler = move |Query(query): Query<BTreeMap<String, String>>, headers: HeaderMap| {
            let spec = spec.clone();
            let cache = Arc::clone(&cache);
            async move {
                let response = match &spec {
                    ResponseSpec::Render { status, template } => Response::new(
                        *status,
                        render::render(template, start.elapsed().as_secs_f64(), seed),
                    ),
                    ResponseSpec::Static(response) => response.clone(),
                    ResponseSpec::Computed(responder) => responder(&RequestCtx {
                        query,
                        t_s: start.elapsed().as_secs_f64(),
                        seed,
                        host: host_of(&headers),
                        cache,
                    }),
                };
                into_http(response)
            }
        };
        let method_router = match endpoint.method.to_ascii_uppercase().as_str() {
            "GET" => routing::get(handler),
            "POST" => routing::post(handler),
            "PUT" => routing::put(handler),
            "DELETE" => routing::delete(handler),
            other => bail!("unsupported HTTP method {other} for {}", endpoint.path),
        };
        router = router.route(&endpoint.path, method_router);
    }
    Ok(router)
}

/// The one place a described response becomes an HTTP one.
fn into_http(response: Response) -> axum::response::Response {
    let status = response.status.code();
    match response.data {
        ResponseData::Json(json) => (status, Json(json)).into_response(),
        ResponseData::Bytes { content_type, data } => {
            let headers = [(header::CONTENT_TYPE, content_type)];
            (status, headers, data.to_vec()).into_response()
        }
    }
}

fn host_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .map(str::to_owned)
}

/// A port held open for a resource, with the router that will answer on it.
#[derive(Debug)]
pub struct Bound {
    port: u16,
    listener: tokio::net::TcpListener,
    router: Router,
}

/// Take `0.0.0.0:port` for `endpoints`.
///
/// Binding is split from serving so the caller can hold the port
/// before advertising the device: a taken port must stop the run,
/// rather than surface from a detached task once mDNS has published it.
pub async fn bind(
    port: u16,
    endpoints: Vec<EndpointSpec>,
    seed: u64,
    start: Instant,
    cache: Arc<Cache>,
) -> Result<Bound> {
    let router = build_router(endpoints, seed, start, &cache)?;
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .with_context(|| format!("binding 0.0.0.0:{port}"))?;
    Ok(Bound {
        port,
        listener,
        router,
    })
}

impl Bound {
    /// Answer on the held port until the task is dropped or the listener errors.
    pub async fn serve(self) -> Result<()> {
        axum::serve(self.listener, self.router)
            .await
            .with_context(|| format!("serving on port {}", self.port))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use axum::body::{Body as AxumBody, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    use crate::blueprint::Instance;
    use crate::cache::Cache;
    use crate::sampler::backfill;

    /// Drive a real GET through the axeos router and assert the response shape.
    /// Structural only — values come from seeded noise, so exact numbers vary.
    async fn get(instance: &Instance, path: &str) -> (StatusCode, serde_json::Value) {
        let resource = instance.resource("axeos-01", 0);
        let series = resource
            .sampler
            .as_ref()
            .map(|s| {
                s.series
                    .iter()
                    .map(|x| (x.name.clone(), x.capacity))
                    .collect()
            })
            .unwrap_or_default();
        let cache = Arc::new(Cache::new::<Vec<_>>(series));
        if let Some(sampler) = resource.sampler.as_ref() {
            backfill(&cache, sampler, 0x51ACE);
        }
        let router = super::build_router(resource.endpoints, 0x51ACE, Instant::now(), &cache)
            .expect("router builds");
        let request = Request::get(path)
            .body(AxumBody::empty())
            .expect("request builds");
        let response = router.oneshot(request).await.expect("router responds");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        (
            status,
            serde_json::from_slice(&bytes).expect("body is json"),
        )
    }

    fn axeos() -> Instance {
        Instance::Axeos {
            label: None,
            params: crate::devices::axeos::Params::default(),
            count: 1,
            port: None,
        }
    }

    /// Bind the axeos resource on `port`, as `serve` does before announcing.
    async fn bind_axeos(port: u16) -> anyhow::Result<super::Bound> {
        let resource = axeos().resource("axeos-01", port);
        super::bind(
            port,
            resource.endpoints,
            0,
            Instant::now(),
            Arc::new(Cache::new::<Vec<_>>(Vec::new())),
        )
        .await
    }

    #[tokio::test]
    async fn a_taken_port_fails_the_bind_rather_than_the_detached_task() {
        // The caller announces only once this returns: a port it cannot hold
        // must fail here, not from the task with the device already advertised.
        let held = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("BUG: an ephemeral port must bind");
        let taken = held
            .local_addr()
            .expect("BUG: bound socket has an address")
            .port();

        let err = bind_axeos(taken)
            .await
            .expect_err("BUG: binding a taken port must fail");
        assert!(
            format!("{err:#}").contains(&taken.to_string()),
            "the failure must name the port: {err:#}"
        );
    }

    #[tokio::test]
    async fn a_free_port_binds_before_anything_is_announced() {
        assert!(bind_axeos(0).await.is_ok(), "port 0 takes any free port");
    }

    #[tokio::test]
    async fn statistics_endpoint_serves_the_accumulated_matrix() {
        let (status, body) = get(&axeos(), "/api/system/statistics").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["labels"],
            serde_json::json!(["hashrate", "power", "asicTemp", "timestamp"]),
        );
        let rows = body["statistics"].as_array().expect("statistics array");
        assert_eq!(rows.len(), 300, "full 10-minute window");
        assert!(
            rows.iter()
                .all(|r| r.as_array().is_some_and(|c| c.len() == 4))
        );
        assert!(body["currentTimestamp"].is_number());
    }

    #[tokio::test]
    async fn a_bytes_endpoint_serves_its_payload_under_its_own_content_type() {
        let endpoint = crate::blueprint::EndpointSpec {
            method: "GET".to_owned(),
            path: "/img/logo/ferrari.png".to_owned(),
            response: crate::blueprint::ResponseSpec::Static(crate::blueprint::Response::ok(
                crate::blueprint::ResponseData::bytes(
                    "image/png",
                    Arc::<[u8]>::from(b"\x89PNG\r\n\x1a\n".as_slice()),
                ),
            )),
        };
        let cache = Arc::new(Cache::new::<Vec<_>>(Vec::new()));
        let router =
            super::build_router(vec![endpoint], 0, Instant::now(), &cache).expect("router builds");
        let request = Request::get("/img/logo/ferrari.png")
            .body(AxumBody::empty())
            .expect("request builds");
        let response = router.oneshot(request).await.expect("router responds");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("image/png"),
            "a binary body must not be labelled as the JSON the other endpoints serve"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        assert_eq!(bytes.as_ref(), b"\x89PNG\r\n\x1a\n");
    }

    #[tokio::test]
    async fn info_endpoint_renders_the_live_leaves() {
        let (status, body) = get(&axeos(), "/api/system/info").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["hashRate"].is_number(), "rendered $value leaf");
        assert!(body["expectedHashrate"].is_number(), "nominal present");
        assert_eq!(body["ASICModel"], "BM1370");
    }
}
