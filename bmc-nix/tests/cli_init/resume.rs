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

use std::sync::Mutex;

use super::*;
use crate::scripted::{Reply, ScriptedServer, serve_scripted};

pub(crate) fn fixture(
    env: &InitEnv,
    version: &str,
    signature: Option<String>,
    replies: Vec<Reply>,
) -> ScriptedServer {
    fixture_with_feed_failures(env, version, signature, Vec::new(), replies)
}

fn fixture_with_feed_failures(
    env: &InitEnv,
    version: &str,
    signature: Option<String>,
    feed_failures: Vec<Reply>,
    replies: Vec<Reply>,
) -> ScriptedServer {
    let pending = bind_server();
    let base = pending.base_url();
    let feed = package_feed_bytes(vec![PackageFeedEntry {
        bos_version: version.to_owned(),
        download_url: format!("{base}/init.tar.gz"),
        profile_path: PROFILE_PATH.to_owned(),
        index_url: None,
        signature,
    }]);
    env.write_servers_config(&base);
    serve_scripted(pending.listener, pending.addr, feed, feed_failures, replies)
}

fn seed_partial(env: &InitEnv, bytes: &[u8], signature: &str) {
    seed_partial_with_metadata(env, bytes, &serde_json::json!({ "signature": signature }));
}

pub(crate) fn seed_partial_with_metadata(
    env: &InitEnv,
    bytes: &[u8],
    metadata: &serde_json::Value,
) {
    std::fs::write(env.download_dir.join("init-tarball.tar.gz.part"), bytes)
        .expect("BUG: seed partial");
    std::fs::write(
        env.download_dir.join("init-tarball.tar.gz.metadata.json"),
        metadata.to_string(),
    )
    .expect("BUG: seed metadata");
}

#[expect(
    clippy::integer_division,
    reason = "any split point makes a valid prefix"
)]
pub(crate) fn half(bytes: usize) -> usize {
    bytes / 2
}

#[derive(Default)]
struct RecordedProgress(Mutex<Vec<usize>>);

impl RecordedProgress {
    fn events(&self) -> Vec<usize> {
        self.0.lock().expect("BUG: progress lock poisoned").clone()
    }
}

impl DownloadProgress for RecordedProgress {
    fn on_bytes_downloaded(&self, downloaded: usize, _: Option<usize>) {
        self.0
            .lock()
            .expect("BUG: progress lock poisoned")
            .push(downloaded);
    }

    fn on_extracting(&self) {}
}

async fn run_signed_init(
    env: &InitEnv,
    server: &ScriptedServer,
    version: &str,
    progress: &RecordedProgress,
) {
    let (_, public) = test_init_keypair();
    run_init_store(
        &env.tmp,
        server.base_url(),
        version,
        &SignatureVerification::Enabled {
            trusted_public_key: public,
        },
        Some(progress),
    )
    .await
    .expect("signed in-process init must succeed");
}

#[test]
fn short_body_resumes_in_the_same_invocation() {
    let env = setup();
    let version = "short-body-resume";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/resumed/marker", b"complete")]);
    let split = half(tarball.len());
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![
            Reply::short(&tarball[..split], tarball.len()),
            Reply::range(&tarball[split..], split, tarball.len()),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(run.status.success(), "init failed: {}", run.stderr);
    assert_eq!(server.ranges(), vec![None, Some(format!("bytes={split}-"))]);
    assert_no_download_artifacts(&env.download_dir, "successful resumed init");
}

#[test]
fn damaged_retained_prefix_gets_one_clean_signed_retry() {
    let env = setup();
    let version = "damaged-prefix";
    let tarball = build_tarball(
        env.tmp.path(),
        &[("nix/store/recovered/marker", b"complete")],
    );
    let signature = sign_with_test_key(&tarball);
    let split = half(tarball.len());
    seed_partial(&env, &vec![0; split], &signature);
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![
            Reply::range(&tarball[split..], split, tarball.len()),
            Reply::full(&tarball),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(run.status.success(), "clean retry failed: {}", run.stderr);
    assert_eq!(server.ranges(), vec![Some(format!("bytes={split}-")), None]);
    assert_no_download_artifacts(&env.download_dir, "successful clean retry");
}

#[test]
fn complete_retained_partial_is_verified_after_416() {
    let env = setup();
    let version = "complete-partial";
    let tarball = build_tarball(
        env.tmp.path(),
        &[("nix/store/complete/marker", b"complete")],
    );
    let signature = sign_with_test_key(&tarball);
    seed_partial(&env, &tarball, &signature);
    let server = fixture(&env, version, Some(signature), vec![Reply::status(416)]);

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "complete partial failed: {}",
        run.stderr
    );
    assert_eq!(
        server.ranges(),
        vec![Some(format!("bytes={}-", tarball.len()))]
    );
    assert_no_download_artifacts(&env.download_dir, "complete retained partial");
}

#[test]
fn changed_signature_discards_retained_partial() {
    let env = setup();
    let version = "changed-signature";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/changed/marker", b"complete")]);
    seed_partial(&env, b"old artifact", "old-signature");
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![Reply::full(&tarball)],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "changed signature failed: {}",
        run.stderr
    );
    assert_eq!(server.ranges(), vec![None]);
    assert_no_download_artifacts(&env.download_dir, "changed signature");
}

#[test]
fn torn_signature_metadata_discards_retained_partial() {
    let env = setup();
    let version = "torn-signature";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/torn/marker", b"complete")]);
    seed_partial(&env, b"stale bytes", "old-signature");
    std::fs::write(
        env.download_dir.join("init-tarball.tar.gz.metadata.json"),
        b"\"incomplete",
    )
    .expect("BUG: tear signature metadata");
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![Reply::full(&tarball)],
    );

    let run = env.run_init(version, &[]);

    assert!(run.status.success(), "torn metadata failed: {}", run.stderr);
    assert_eq!(server.ranges(), vec![None]);
    assert_no_download_artifacts(&env.download_dir, "torn signature metadata");
}

#[test]
fn unsigned_init_never_resumes_retained_partial() {
    let env = setup();
    let version = "unsigned-no-resume";
    let tarball = build_tarball(
        env.tmp.path(),
        &[("nix/store/unsigned/marker", b"complete")],
    );
    seed_partial(&env, b"old artifact", "old-signature");
    let server = fixture(&env, version, None, vec![Reply::full(&tarball)]);

    let run = env.run_init(version, &["--no-verify-signature"]);

    assert!(run.status.success(), "unsigned init failed: {}", run.stderr);
    assert_eq!(server.ranges(), vec![None]);
    assert_no_download_artifacts(&env.download_dir, "unsigned init");
}

// Five dropped connections walk the real RETRY_DELAYS: about 15 s of sleeping.
#[test]
fn exhausted_transfer_resumes_in_a_later_process() {
    let env = setup();
    let version = "restart-resume";
    let tarball = build_tarball(
        env.tmp.path(),
        &[("nix/store/restarted/marker", b"complete")],
    );
    let split = half(tarball.len());
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![
            Reply::short(&tarball[..split], tarball.len()),
            Reply::Drop,
            Reply::Drop,
            Reply::Drop,
            Reply::Drop,
            Reply::Drop,
            Reply::range(&tarball[split..], split, tarball.len()),
        ],
    );

    let first = env.run_init(version, &[]);
    assert!(!first.status.success(), "first run unexpectedly succeeded");
    assert_eq!(
        std::fs::metadata(env.download_dir.join("init-tarball.tar.gz.part"))
            .expect("retained partial")
            .len(),
        split as u64
    );
    assert!(
        env.download_dir
            .join("init-tarball.tar.gz.metadata.json")
            .is_file(),
        "retry exhaustion must retain the signature for the next process"
    );

    let second = env.run_init(version, &[]);

    assert!(second.status.success(), "restart failed: {}", second.stderr);
    let ranges = server.ranges();
    assert_eq!(ranges.len(), 7);
    assert_eq!(ranges[0], None);
    assert!(
        ranges[1..]
            .iter()
            .all(|range| range.as_deref() == Some(format!("bytes={split}-").as_str())),
        "every request after the interrupted body should resume: {ranges:?}"
    );
    assert_no_download_artifacts(&env.download_dir, "successful later process");
}

#[test]
fn progressing_interruptions_continue_past_the_no_progress_limit() {
    let env = setup();
    let version = "progressing-interruptions";
    let tarball = build_tarball(
        env.tmp.path(),
        &[("nix/store/progressing/marker", b"complete")],
    );
    let mut replies = vec![Reply::short(&tarball[..1], tarball.len())];
    for offset in 1..5 {
        replies.push(Reply::short_range(
            &tarball[offset..=offset],
            offset,
            tarball.len(),
        ));
    }
    replies.push(Reply::range(&tarball[5..], 5, tarball.len()));
    let server = fixture(&env, version, Some(sign_with_test_key(&tarball)), replies);

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "progressing download failed: {}",
        run.stderr
    );
    assert_eq!(server.ranges().len(), 6);
    assert_no_download_artifacts(&env.download_dir, "progressing download");
}

#[test]
fn ignored_range_replaces_the_partial_from_zero() {
    let env = setup();
    let version = "ignored-range";
    let tarball = build_tarball(
        env.tmp.path(),
        &[("nix/store/replaced/marker", b"complete")],
    );
    let signature = sign_with_test_key(&tarball);
    seed_partial(&env, b"obsolete prefix", &signature);
    let server = fixture(&env, version, Some(signature), vec![Reply::full(&tarball)]);

    let run = env.run_init(version, &[]);

    assert!(run.status.success(), "ignored range failed: {}", run.stderr);
    assert_eq!(server.ranges(), vec![Some("bytes=15-".to_owned())]);
    assert_no_download_artifacts(&env.download_dir, "ignored range");
}

#[test]
fn bad_clean_retry_never_reaches_extraction() {
    let env = setup();
    let version = "second-bad-signature";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/reject/marker", b"complete")]);
    let signature = sign_with_test_key(&tarball);
    let split = half(tarball.len());
    seed_partial(&env, &vec![0; split], &signature);
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![
            Reply::range(&tarball[split..], split, tarball.len()),
            Reply::full(b"also invalid"),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(
        !run.status.success(),
        "bad clean retry unexpectedly succeeded"
    );
    assert!(run.stderr.contains("signature verification failed"));
    assert_eq!(server.ranges(), vec![Some(format!("bytes={split}-")), None]);
    assert_no_download_artifacts(&env.download_dir, "two bad signed candidates");
    assert!(
        !env.data_dir.join("nix").exists(),
        "unverified data was extracted"
    );
}

#[test]
fn bad_fresh_download_does_not_redownload() {
    let env = setup();
    let version = "bad-fresh-download";
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(b"expected tarball")),
        vec![Reply::full(b"corrupt tarball")],
    );

    let run = env.run_init(version, &[]);

    assert!(!run.status.success(), "bad signature unexpectedly accepted");
    assert!(run.stderr.contains("signature verification failed"));
    assert_eq!(server.ranges(), vec![None]);
    assert_no_download_artifacts(&env.download_dir, "rejected fresh download");
}

#[test]
fn definitive_tarball_status_does_not_retry() {
    let env = setup();
    let version = "definitive-status";
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(b"unused")),
        vec![Reply::status(404)],
    );

    let run = env.run_init(version, &[]);

    assert!(!run.status.success(), "404 unexpectedly succeeded");
    assert_eq!(server.ranges(), vec![None]);
}

#[test]
fn verified_final_is_reused_after_extraction_failure() {
    let env = setup();
    let version = "final-reuse";
    let bytes = b"signed, but not a gzip stream";
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(bytes)),
        vec![Reply::full(bytes)],
    );

    let first = env.run_init(version, &[]);
    assert!(
        !first.status.success(),
        "invalid gzip unexpectedly extracted"
    );
    assert!(env.download_dir.join("init-tarball.tar.gz").is_file());
    let second = env.run_init(version, &[]);

    assert!(
        !second.status.success(),
        "invalid gzip unexpectedly extracted"
    );
    assert_eq!(
        server.ranges(),
        vec![None],
        "verified file should be reused"
    );
}

#[test]
fn manually_placed_valid_final_skips_download() {
    let env = setup();
    let version = "manual-valid-final";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/manual/marker", b"complete")]);
    std::fs::write(env.download_dir.join("init-tarball.tar.gz"), &tarball)
        .expect("BUG: seed final tarball");
    let server = fixture(&env, version, Some(sign_with_test_key(&tarball)), vec![]);

    let run = env.run_init(version, &[]);

    assert!(run.status.success(), "valid final failed: {}", run.stderr);
    assert!(
        server.ranges().is_empty(),
        "valid final was downloaded again"
    );
    assert_no_download_artifacts(&env.download_dir, "manually placed valid final");
}

#[test]
fn manually_placed_invalid_final_gets_one_clean_download() {
    let env = setup();
    let version = "manual-invalid-final";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/manual/marker", b"complete")]);
    std::fs::write(
        env.download_dir.join("init-tarball.tar.gz"),
        b"invalid tarball",
    )
    .expect("BUG: seed invalid final tarball");
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![Reply::full(&tarball)],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "clean download failed: {}",
        run.stderr
    );
    assert_eq!(server.ranges(), vec![None]);
    assert_no_download_artifacts(&env.download_dir, "manually placed invalid final");
}

/// A stale final tarball says nothing about the server,
/// so a damaged resume later in the same run still gets its clean retry.
#[test]
fn invalid_final_leaves_the_clean_retry_for_a_damaged_resume() {
    let env = setup();
    let version = "invalid-final-then-damaged-resume";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/stale/marker", b"complete")]);
    let split = half(tarball.len());
    std::fs::write(
        env.download_dir.join("init-tarball.tar.gz"),
        b"stale tarball",
    )
    .expect("BUG: seed invalid final tarball");
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![
            Reply::short(&vec![0; split], tarball.len()),
            Reply::range(&tarball[split..], split, tarball.len()),
            Reply::full(&tarball),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "clean retry was spent: {}",
        run.stderr
    );
    assert_eq!(
        server.ranges(),
        vec![None, Some(format!("bytes={split}-")), None]
    );
    assert_no_download_artifacts(&env.download_dir, "stale final and damaged resume");
}

#[test]
fn directory_in_place_of_the_partial_is_removed() {
    let env = setup();
    let version = "directory-partial";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/dir/marker", b"complete")]);
    let part = env.download_dir.join("init-tarball.tar.gz.part");
    std::fs::create_dir(&part).expect("BUG: create directory partial");
    std::fs::write(part.join("leftover"), b"leftover").expect("BUG: seed directory content");
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![Reply::full(&tarball)],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "directory blocked init: {}",
        run.stderr
    );
    assert_eq!(server.ranges(), vec![None]);
    assert_no_download_artifacts(&env.download_dir, "directory partial");
}

#[test]
fn transient_feed_and_tarball_statuses_retry_independently() {
    let env = setup();
    let version = "transient-statuses";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/retried/marker", b"complete")]);
    let server = fixture_with_feed_failures(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![Reply::status(503)],
        vec![Reply::status(503), Reply::full(&tarball)],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "transient status failed: {}",
        run.stderr
    );
    assert_eq!(server.feed_hits(), 2);
    assert_eq!(server.ranges(), vec![None, None]);
    assert_no_download_artifacts(&env.download_dir, "transient statuses");
}

#[test]
fn malformed_range_response_falls_back_to_a_plain_request() {
    let env = setup();
    let version = "malformed-range";
    let tarball = build_tarball(
        env.tmp.path(),
        &[("nix/store/fallback/marker", b"complete")],
    );
    let signature = sign_with_test_key(&tarball);
    let prefix = b"old partial";
    seed_partial(&env, prefix, &signature);
    let wrong_range = Reply::Http {
        status: 206,
        declared_len: Some(tarball.len() - prefix.len()),
        headers: vec![(
            "Content-Range".to_owned(),
            format!(
                "bytes 0-{}/{total}",
                tarball.len() - prefix.len() - 1,
                total = tarball.len()
            ),
        )],
        body: tarball[prefix.len()..].to_vec(),
    };
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![wrong_range, Reply::full(&tarball)],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "plain fallback failed: {}",
        run.stderr
    );
    assert_eq!(
        server.ranges(),
        vec![Some(format!("bytes={}-", prefix.len())), None]
    );
    assert_no_download_artifacts(&env.download_dir, "malformed range fallback");
}

#[test]
fn damaged_complete_partial_on_416_gets_one_clean_retry() {
    let env = setup();
    let version = "damaged-complete-partial";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/416/marker", b"complete")]);
    let signature = sign_with_test_key(&tarball);
    seed_partial(&env, &vec![0; tarball.len()], &signature);
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![Reply::status(416), Reply::full(&tarball)],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "clean retry after 416 failed: {}",
        run.stderr
    );
    assert_eq!(
        server.ranges(),
        vec![Some(format!("bytes={}-", tarball.len())), None]
    );
    assert_no_download_artifacts(&env.download_dir, "416 clean retry");
}

#[tokio::test]
#[serial]
async fn resumed_transfer_reports_progress_from_the_retained_offset() {
    let env = setup();
    let version = "resumed-progress";
    let tarball = build_tarball(
        env.tmp.path(),
        &[("nix/store/progress/marker", b"complete")],
    );
    let signature = sign_with_test_key(&tarball);
    let split = half(tarball.len());
    seed_partial(&env, &tarball[..split], &signature);
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![Reply::range(&tarball[split..], split, tarball.len())],
    );
    let progress = RecordedProgress::default();

    run_signed_init(&env, &server, version, &progress).await;

    let events = progress.events();
    assert_eq!(events.first(), Some(&split), "{events:?}");
    assert_eq!(events.last(), Some(&tarball.len()), "{events:?}");
    assert!(
        !events.contains(&0),
        "an honoured range must not restart progress: {events:?}"
    );
}

#[tokio::test]
#[serial]
async fn ignored_range_restarts_reported_progress() {
    let env = setup();
    let version = "restarted-progress";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/restart/marker", b"complete")]);
    let signature = sign_with_test_key(&tarball);
    let split = half(tarball.len());
    seed_partial(&env, &tarball[..split], &signature);
    let server = fixture(&env, version, Some(signature), vec![Reply::full(&tarball)]);
    let progress = RecordedProgress::default();

    run_signed_init(&env, &server, version, &progress).await;

    let events = progress.events();
    assert_eq!(events.first(), Some(&split), "{events:?}");
    assert_eq!(events.get(1), Some(&0), "{events:?}");
    assert_eq!(events.last(), Some(&tarball.len()), "{events:?}");
}

/// A ranged request the server answers with `rejection` must give way to
/// one plain request that completes the download.
fn assert_ranged_rejection_falls_back_to_plain(version: &str, rejection: Reply) {
    let env = setup();
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/plain/marker", b"complete")]);
    let signature = sign_with_test_key(&tarball);
    let prefix = b"retained prefix";
    seed_partial(&env, prefix, &signature);
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![rejection, Reply::full(&tarball)],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "plain fallback failed: {}",
        run.stderr
    );
    assert_eq!(
        server.ranges(),
        vec![Some(format!("bytes={}-", prefix.len())), None]
    );
    assert_no_download_artifacts(&env.download_dir, "ranged request rejected");
}

#[test]
fn forbidden_ranged_request_falls_back_to_a_plain_request() {
    assert_ranged_rejection_falls_back_to_plain("ranged-forbidden", Reply::status(403));
}

#[test]
fn not_implemented_ranged_request_falls_back_to_a_plain_request() {
    assert_ranged_rejection_falls_back_to_plain("ranged-not-implemented", Reply::status(501));
}

#[test]
fn bodiless_success_to_a_ranged_request_falls_back_to_a_plain_request() {
    assert_ranged_rejection_falls_back_to_plain("ranged-no-content", Reply::status(204));
}

#[test]
fn close_delimited_response_fails_a_signed_download() {
    let env = setup();
    let version = "close-delimited";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/close/marker", b"complete")]);
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![Reply::close_delimited(&tarball)],
    );

    let run = env.run_init(version, &[]);

    assert!(
        !run.status.success(),
        "close-delimited 200 unexpectedly accepted"
    );
    assert!(
        run.stderr
            .contains("neither Content-Length nor Transfer-Encoding"),
        "{}",
        run.stderr
    );
    assert_eq!(server.ranges(), vec![None]);
    assert_no_download_artifacts(&env.download_dir, "close-delimited response");
}

#[test]
fn changed_total_length_discards_retained_partial() {
    let env = setup();
    let version = "changed-total";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/total/marker", b"complete")]);
    let signature = sign_with_test_key(&tarball);
    let split = half(tarball.len());
    seed_partial_with_metadata(
        &env,
        &tarball[..split],
        &serde_json::json!({ "signature": signature, "total": tarball.len() + 1 }),
    );
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![
            Reply::range(&tarball[split..], split, tarball.len()),
            Reply::full(&tarball),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(run.status.success(), "changed total failed: {}", run.stderr);
    assert_eq!(server.ranges(), vec![Some(format!("bytes={split}-")), None]);
    assert_no_download_artifacts(&env.download_dir, "changed total length");
}

#[test]
fn partial_survives_a_plain_request_that_fails_too() {
    let env = setup();
    let version = "refused-twice";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/refused/marker", b"complete")]);
    let signature = sign_with_test_key(&tarball);
    let split = half(tarball.len());
    seed_partial(&env, &tarball[..split], &signature);
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![Reply::status(404), Reply::status(404)],
    );

    let run = env.run_init(version, &[]);

    assert!(
        !run.status.success(),
        "a 404 to the plain request must fail"
    );
    assert_eq!(server.ranges(), vec![Some(format!("bytes={split}-")), None]);
    assert_eq!(
        std::fs::read(env.download_dir.join("init-tarball.tar.gz.part")).expect("retained partial"),
        &tarball[..split],
        "a refusal that the plain request repeats proves nothing against the retained bytes"
    );
}

#[test]
fn ranged_requests_the_server_keeps_failing_fall_back_to_a_plain_request() {
    let env = setup();
    let version = "ranged-unavailable";
    let tarball = build_tarball(
        env.tmp.path(),
        &[("nix/store/unavailable/marker", b"complete")],
    );
    let signature = sign_with_test_key(&tarball);
    let split = half(tarball.len());
    seed_partial(&env, &tarball[..split], &signature);
    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![
            Reply::status(503),
            Reply::status(503),
            Reply::full(&tarball),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "a server that fails only ranged requests pins every run to the partial: {}",
        run.stderr
    );
    let ranged = Some(format!("bytes={split}-"));
    assert_eq!(
        server.ranges(),
        vec![ranged.clone(), ranged, None],
        "one ranged error alone must not give up the partial"
    );
}

#[test]
fn transport_errors_do_not_count_toward_giving_up_the_range() {
    let env = setup();
    let version = "ranged-blip";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/blip/marker", b"complete")]);
    let signature = sign_with_test_key(&tarball);
    let split = half(tarball.len());

    seed_partial(&env, &tarball[..split], &signature);

    let server = fixture(
        &env,
        version,
        Some(signature),
        vec![
            Reply::Drop,
            Reply::status(503),
            Reply::range(&tarball[split..], split, tarball.len()),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(
        run.status.success(),
        "a network blip and one ranged error must still resume: {}",
        run.stderr
    );

    let ranged = Some(format!("bytes={split}-"));

    assert_eq!(
        server.ranges(),
        vec![ranged.clone(), ranged.clone(), ranged],
        "a network blip says nothing about the server and must not cost the partial"
    );
}

/// Chunked-encodes `body` without the terminating chunk,
/// so the transfer breaks after it.
fn truncated_chunked(body: &[u8]) -> Reply {
    let mut encoded = format!("{:x}\r\n", body.len()).into_bytes();
    encoded.extend_from_slice(body);
    encoded.extend_from_slice(b"\r\n");
    Reply::Http {
        status: 200,
        declared_len: None,
        headers: vec![("Transfer-Encoding".to_owned(), "chunked".to_owned())],
        body: encoded,
    }
}

#[test]
fn total_from_a_range_response_is_kept_for_the_next_run() {
    let env = setup();
    let version = "chunked-total";
    let tarball = build_tarball(env.tmp.path(), &[("nix/store/chunked/marker", b"complete")]);
    let quarter = half(half(tarball.len()));
    let split = half(tarball.len());
    let server = fixture(
        &env,
        version,
        Some(sign_with_test_key(&tarball)),
        vec![
            truncated_chunked(&tarball[..quarter]),
            Reply::short_range(&tarball[quarter..split], quarter, tarball.len()),
            Reply::status(404),
            Reply::status(404),
        ],
    );

    let run = env.run_init(version, &[]);

    assert!(
        !run.status.success(),
        "a 404 to the plain request must fail"
    );
    let metadata: serde_json::Value = serde_json::from_slice(
        &std::fs::read(env.download_dir.join("init-tarball.tar.gz.metadata.json"))
            .expect("retained metadata"),
    )
    .expect("BUG: metadata is JSON");
    assert_eq!(
        metadata["total"],
        tarball.len(),
        "the next run can check the total only if the range response's total was kept"
    );
    assert_eq!(server.ranges().len(), 4);
}
