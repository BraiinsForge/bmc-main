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

use super::*;
use crate::resume::{fixture, half, seed_partial_with_metadata};
use crate::scripted::Reply;

/// A recorded tag that is not a strong validator must not reach `If-Range`,
/// and must not keep the resume from happening either.
fn assert_recorded_entity_tag_is_left_out(version: &str, etag: &str) {
    let env = setup();
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/badetag/marker", b"complete")]);
    let signature = sign_with_test_key(&tarball);
    let split = half(tarball.len());
    seed_partial_with_metadata(
        &env,
        &tarball[..split],
        &serde_json::json!({ "signature": signature, "etag": etag }),
    );
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![Reply::range(&tarball[split..], split, tarball.len())],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "resume with a bad etag failed: {}",
        run.stderr
    );
    assert_eq!(server.ranges(), vec![Some(format!("bytes={split}-"))]);
    assert_eq!(server.if_ranges(), vec![None]);
}

#[test]
fn unparsable_recorded_entity_tag_is_left_out_of_the_resume_request() {
    assert_recorded_entity_tag_is_left_out("bad-etag", "\n");
}

#[test]
fn weak_recorded_entity_tag_is_left_out_of_the_resume_request() {
    assert_recorded_entity_tag_is_left_out("weak-recorded-etag", "W/\"v1\"");
}

#[test]
fn resume_requests_carry_the_recorded_entity_tag() {
    let env = setup();
    let version = "etag-resume";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/etag/marker", b"complete")]);
    let split = half(tarball.len());
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![
            Reply::short(&tarball[..split], tarball.len()).with_header("ETag", "\"v1\""),
            Reply::range(&tarball[split..], split, tarball.len()),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(run.status.success(), "etag resume failed: {}", run.stderr);
    assert_eq!(server.if_ranges(), vec![None, Some("\"v1\"".to_owned())]);
}

#[test]
fn weak_entity_tags_are_not_sent_as_if_range() {
    let env = setup();
    let version = "weak-etag";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/weak/marker", b"complete")]);
    let split = half(tarball.len());
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![
            Reply::short(&tarball[..split], tarball.len()).with_header("ETag", "W/\"v1\""),
            Reply::range(&tarball[split..], split, tarball.len()),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "weak etag resume failed: {}",
        run.stderr
    );
    assert_eq!(server.if_ranges(), vec![None, None]);
}

#[test]
fn range_response_for_another_entity_tag_falls_back_to_a_plain_request() {
    let env = setup();
    let version = "changed-etag";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/changed/marker", b"complete")]);
    let signature = sign_with_test_key(&tarball);
    let split = half(tarball.len());
    seed_partial_with_metadata(
        &env,
        &tarball[..split],
        &serde_json::json!({ "signature": signature, "etag": "\"v1\"" }),
    );
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![
            Reply::range(&tarball[split..], split, tarball.len()).with_header("ETag", "\"v2\""),
            Reply::full(&tarball),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(run.status.success(), "changed etag failed: {}", run.stderr);
    assert_eq!(
        server.ranges(),
        vec![Some(format!("bytes={split}-")), None],
        "a range of another entity must not be appended to the partial"
    );
    assert_no_download_artifacts(&env.download_dir, "changed entity tag");
}
