//! x_archive importer (P3-IMP-02): the X/Twitter "download your data" zip.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};

use common::*;
use openstance_import::x_archive::XArchiveImporter;
use openstance_import::*;

fn imp() -> XArchiveImporter {
    XArchiveImporter::new(ImportOptions::default())
}

/// Zip `(name, bytes)` entries into `dir/name` (deflated).
fn zip_entries(dir: &Path, name: &str, entries: &[(&str, Vec<u8>)]) -> PathBuf {
    let p = dir.join(name);
    let f = std::fs::File::create(&p).unwrap();
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (n, b) in entries {
        z.start_file(*n, o).unwrap();
        z.write_all(b).unwrap();
    }
    z.finish().unwrap();
    p
}

/// The committed unzipped fixture, zipped into a temp dir.
fn fixture_zip(dir: &Path) -> PathBuf {
    let root = fixture("x/archive");
    let mut entries = Vec::new();
    for e in std::fs::read_dir(root.join("data")).unwrap() {
        let e = e.unwrap();
        let name = format!("data/{}", e.file_name().to_string_lossy());
        entries.push((name, std::fs::read(e.path()).unwrap()));
    }
    entries.sort();
    let refs: Vec<(&str, Vec<u8>)> = entries
        .iter()
        .map(|(n, b)| (n.as_str(), b.clone()))
        .collect();
    zip_entries(dir, "twitter-archive.zip", &refs)
}

const MANIFEST: &str = r#"window.__THAR_CONFIG = {
  "userInfo" : { "accountId" : "100", "userName" : "owner_x", "displayName" : "Owner" },
  "archiveInfo" : { "generationDate" : "2026-09-30T12:00:00.000Z", "isPartialArchive" : false }
}"#;

fn following(ids: &[&str]) -> String {
    let items: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                r#"{{ "following" : {{ "accountId" : "{id}", "userLink" : "https://twitter.com/intent/user?user_id={id}" }} }}"#
            )
        })
        .collect();
    format!(
        "window.YTD.following.part0 = [\n  {}\n]",
        items.join(",\n  ")
    )
}

fn by_kind(sink: &MemorySink, kind: InteractionKind) -> Vec<&RawInteraction> {
    sink.interactions
        .iter()
        .filter(|i| i.kind == kind)
        .collect()
}

fn ids(i: &RawInteraction) -> Vec<(Role, String)> {
    i.participants
        .iter()
        .flat_map(|p| p.handles.iter().map(move |h| (p.role, h.raw.clone())))
        .collect()
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn detect_zip_strong_and_ytd_js_weak() {
    let dir = tempfile::tempdir().unwrap();
    let good = zip_entries(
        dir.path(),
        "archive.zip",
        &[
            ("data/manifest.js", MANIFEST.into()),
            ("data/following.js", following(&["1"]).into()),
        ],
    );
    assert_eq!(imp().detect(&good), Detection::Strong);
    assert_eq!(imp().kind(), "x_archive");
    // A zip without the manifest, or with only the manifest, is not ours.
    let other = zip_entries(
        dir.path(),
        "other.zip",
        &[("data/following.js", following(&["1"]).into())],
    );
    assert_eq!(imp().detect(&other), Detection::No);
    let bare = zip_entries(
        dir.path(),
        "bare.zip",
        &[("data/manifest.js", MANIFEST.into())],
    );
    assert_eq!(imp().detect(&bare), Detection::No);
    // A single data file.
    let (_d, js) = temp_file("following.js", following(&["1"]).as_bytes());
    assert_eq!(imp().detect(&js), Detection::Weak);
    let (_d2, plain) = temp_file("app.js", b"console.log(1)");
    assert_eq!(imp().detect(&plain), Detection::No);
    assert_eq!(imp().detect(&dir.path().join("missing.zip")), Detection::No);
    let (_d3, notzip) = temp_file("broken.zip", b"PK\x03\x04 not really");
    assert_eq!(imp().detect(&notzip), Detection::No);
}

#[test]
fn detect_unzipped_folder() {
    let root = fixture("x/archive");
    assert_eq!(imp().detect(&root), Detection::Strong);
    assert_eq!(imp().detect(&fixture("vcf")), Detection::No);
}

// ---------------------------------------------------------------------------
// Follows
// ---------------------------------------------------------------------------

#[test]
fn follows_from_a_zip() {
    let dir = tempfile::tempdir().unwrap();
    let follower = r#"window.YTD.follower.part0 = [ {
  "follower" : { "accountId" : "300", "userLink" : "https://twitter.com/intent/user?user_id=300" }
} ]"#;
    let p = zip_entries(
        dir.path(),
        "a.zip",
        &[
            ("data/manifest.js", MANIFEST.into()),
            ("data/following.js", following(&["200", "201"]).into()),
            ("data/follower.js", follower.into()),
        ],
    );
    let (stats, sink) = run(&imp(), &p);
    assert_eq!(stats.records_read, 3);
    let f = by_kind(&sink, InteractionKind::Follow);
    assert_eq!(f.len(), 3);
    let gen = utc(2026, 9, 30, 12, 0, 0);
    let got: Vec<_> = f
        .iter()
        .map(|i| {
            (
                i.external_id.clone().unwrap(),
                i.dir_hint,
                i.at,
                i.subject.clone(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (
                "follow:out:200".to_string(),
                Some(Direction::Out),
                gen,
                None
            ),
            (
                "follow:out:201".to_string(),
                Some(Direction::Out),
                gen,
                None
            ),
            ("follow:in:300".to_string(), Some(Direction::In), gen, None),
        ]
    );
    assert_eq!(ids(f[0]), [(Role::Subject, "200".to_string())]);
    assert_eq!(f[0].participants[0].handles[0].kind, HandleKind::XUserId);
    assert!(
        f[0].participants[0].hints.is_empty(),
        "the archive has no names"
    );
    assert_eq!(stats.self_candidates, ["x_user_id:100", "x_handle:owner_x"]);
}

#[test]
fn archive_inside_a_top_level_folder_and_multi_part_files() {
    let dir = tempfile::tempdir().unwrap();
    let part1 = following(&["202"]).replace("part0", "part1");
    let p = zip_entries(
        dir.path(),
        "a.zip",
        &[
            ("twitter-2026-09-30/data/manifest.js", MANIFEST.into()),
            ("twitter-2026-09-30/data/following.js", following(&["200", "201"]).into()),
            ("twitter-2026-09-30/data/following-part1.js", part1.into()),
            ("twitter-2026-09-30/data/tweets.js", "window.YTD.tweets.part0 = [ { \"tweet\" : { \"full_text\" : \"SENTINEL-BODY-7f3a\" } } ]".into()),
        ],
    );
    let (_, sink) = run(&imp(), &p);
    let got: Vec<_> = sink
        .interactions
        .iter()
        .map(|i| i.external_id.clone().unwrap())
        .collect();
    assert_eq!(got, ["follow:out:200", "follow:out:201", "follow:out:202"]);
    assert_no_sentinel(&sink);
}

#[test]
fn single_js_file_without_manifest_uses_the_file_time() {
    let (_d, p) = temp_file("following.js", following(&["200"]).as_bytes());
    let mtime = std::fs::metadata(&p)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (stats, sink) = run(&imp(), &p);
    assert_eq!(stats.records_read, 1);
    assert_eq!(sink.interactions[0].at, mtime);
    assert!(stats.self_candidates.is_empty());
}

#[test]
fn malformed_items_are_skipped_and_bad_json_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mixed = r#"window.YTD.following.part0 = [
  { "following" : { "accountId" : "200" } },
  { "following" : { "userLink" : "no id" } },
  { "following" : { "accountId" : "not-a-number" } },
  { "something" : "else" },
  "string item",
  { "following" : { "accountId" : "201", "note": "a ] bracket and \" quote { inside" } }
]"#;
    let p = zip_entries(
        dir.path(),
        "a.zip",
        &[
            ("data/manifest.js", MANIFEST.into()),
            ("data/following.js", mixed.into()),
        ],
    );
    let (stats, sink) = run(&imp(), &p);
    assert_eq!(stats.records_read, 6);
    assert_eq!(stats.skipped.get("malformed"), Some(&4));
    assert_eq!(sink.interactions.len(), 2);

    let broken = "window.YTD.following.part0 = [ { \"following\" : { \"accountId\" : \"200\" ";
    let p = zip_entries(
        dir.path(),
        "b.zip",
        &[
            ("data/manifest.js", MANIFEST.into()),
            ("data/following.js", broken.into()),
        ],
    );
    let err = imp().import(&p, &mut MemorySink::default()).unwrap_err();
    assert!(matches!(err, ImportError::Parse { .. }), "{err}");
}

// ---------------------------------------------------------------------------
// DMs
// ---------------------------------------------------------------------------

const DMS: &str = r#"window.YTD.direct_messages.part0 = [ {
  "dmConversation" : {
    "conversationId" : "100-200",
    "messages" : [ {
      "messageCreate" : {
        "recipientId" : "100", "reactions" : [ ], "urls" : [ ],
        "text" : "SENTINEL-BODY-7f3a", "mediaUrls" : [ ],
        "senderId" : "200", "id" : "1800000000000000001",
        "createdAt" : "2026-08-01T10:00:00.000Z"
      }
    }, {
      "reactionCreate" : { "senderId" : "100", "reactionKey" : "like", "eventId" : "1", "createdAt" : "2026-08-01T10:01:00.000Z" }
    }, {
      "messageCreate" : {
        "recipientId" : "200", "text" : "SENTINEL-BODY-7f3a",
        "senderId" : "100", "id" : "1800000000000000002",
        "createdAt" : "2026-08-01T10:05:00.000Z"
      }
    }, {
      "messageCreate" : { "senderId" : "200", "id" : "1800000000000000003", "createdAt" : "yesterday" }
    } ]
  }
} ]"#;

const GROUP_DMS: &str = r#"window.YTD.direct_messages_group.part0 = [ {
  "dmConversation" : {
    "conversationId" : "1600000000000000001",
    "messages" : [ {
      "messageCreate" : {
        "reactions" : [ ], "urls" : [ ], "text" : "SENTINEL-BODY-7f3a", "mediaUrls" : [ ],
        "senderId" : "300", "id" : "1800000000000000010",
        "createdAt" : "2026-08-05T09:00:00.000Z"
      }
    }, {
      "conversationNameUpdate" : { "initiatingUserId" : "300", "name" : "SENTINEL-BODY-7f3a", "createdAt" : "2026-08-04T09:00:00.000Z" }
    }, {
      "joinConversation" : {
        "initiatingUserId" : "300",
        "participantsSnapshot" : [ "100", "200", "300" ],
        "createdAt" : "2026-08-04T08:00:00.000Z"
      }
    }, {
      "participantsJoin" : { "initiatingUserId" : "300", "userIds" : [ "400" ], "createdAt" : "2026-08-04T08:30:00.000Z" }
    } ]
  }
} ]"#;

#[test]
fn one_to_one_dms_with_direction() {
    let dir = tempfile::tempdir().unwrap();
    let p = zip_entries(
        dir.path(),
        "a.zip",
        &[
            ("data/manifest.js", MANIFEST.into()),
            ("data/direct-messages.js", DMS.into()),
        ],
    );
    let (stats, sink) = run(&imp(), &p);
    let d = by_kind(&sink, InteractionKind::Dm);
    assert_eq!(d.len(), 2);
    assert_eq!(
        stats.records_read, 3,
        "messages read (reactions are not messages)"
    );
    assert_eq!(stats.skipped.get("bad_date"), Some(&1));
    assert_eq!(d[0].external_id.as_deref(), Some("dm:1800000000000000001"));
    assert_eq!(d[0].thread_key.as_deref(), Some("100-200"));
    assert_eq!(d[0].at, utc(2026, 8, 1, 10, 0, 0));
    assert_eq!(d[0].dir_hint, Some(Direction::In));
    assert_eq!(d[0].subject, None);
    assert_eq!(
        ids(d[0]),
        [
            (Role::Sender, "200".to_string()),
            (Role::Recipient, "100".to_string())
        ]
    );
    assert_eq!(d[1].dir_hint, Some(Direction::Out));
    assert_eq!(
        ids(d[1]),
        [
            (Role::Sender, "100".to_string()),
            (Role::Recipient, "200".to_string())
        ]
    );
    assert_no_sentinel(&sink);
}

#[test]
fn dm_without_known_owner_has_no_direction_and_recipient_from_conversation() {
    let dms = DMS.replace(r#""recipientId" : "200", "#, "");
    let (_d, p) = temp_file("direct-messages.js", dms.as_bytes());
    let (_, sink) = run(&imp(), &p);
    let d = by_kind(&sink, InteractionKind::Dm);
    assert_eq!(d.len(), 2);
    assert!(d.iter().all(|i| i.dir_hint.is_none()));
    // The second message lost its recipientId: the other id of the
    // conversation "100-200" stands in.
    assert_eq!(
        ids(d[1]),
        [
            (Role::Sender, "100".to_string()),
            (Role::Recipient, "200".to_string())
        ]
    );
}

#[test]
fn group_dms_have_members() {
    let dir = tempfile::tempdir().unwrap();
    let p = zip_entries(
        dir.path(),
        "a.zip",
        &[
            ("data/manifest.js", MANIFEST.into()),
            ("data/direct-messages-group.js", GROUP_DMS.into()),
        ],
    );
    let (stats, sink) = run(&imp(), &p);
    assert_eq!(stats.records_read, 1);
    let d = by_kind(&sink, InteractionKind::Dm);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].dir_hint, Some(Direction::In));
    assert_eq!(d[0].thread_key.as_deref(), Some("1600000000000000001"));
    assert_eq!(
        ids(d[0]),
        [
            (Role::Sender, "300".to_string()),
            (Role::Member, "100".to_string()),
            (Role::Member, "200".to_string()),
            (Role::Member, "400".to_string()),
        ]
    );
    assert_no_sentinel(&sink);
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

#[test]
fn oversized_entries_abort_the_import() {
    let dir = tempfile::tempdir().unwrap();
    let many: Vec<String> = (0..2000).map(|i| (1000 + i).to_string()).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    let p = zip_entries(
        dir.path(),
        "a.zip",
        &[
            ("data/manifest.js", MANIFEST.into()),
            ("data/following.js", following(&refs).into()),
        ],
    );
    let small = imp().with_limits(16 * 1024, 1024 * 1024);
    let err = small.import(&p, &mut MemorySink::default()).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
    let total = imp().with_limits(1024 * 1024, 16 * 1024);
    let err = total.import(&p, &mut MemorySink::default()).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
    let (stats, _) = run(&imp(), &p);
    assert_eq!(stats.records_read, 2000);
}

// ---------------------------------------------------------------------------
// Committed fixture
// ---------------------------------------------------------------------------

#[test]
fn fixture_archive_counts() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture_zip(dir.path());
    let (stats, sink) = run(&imp(), &p);
    assert_eq!(by_kind(&sink, InteractionKind::Follow).len(), 14);
    assert_eq!(by_kind(&sink, InteractionKind::Dm).len(), 7);
    assert_eq!(sink.interactions.len(), 21);
    assert_eq!(stats.records_read, 21);
    assert_eq!(stats.skipped_total(), 0);
    assert_eq!(
        stats.self_candidates,
        ["x_user_id:1000000001", "x_handle:owenryder_x"]
    );
    let mut distinct = handles(&sink, HandleKind::XUserId);
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 13);
    assert!(sink.identities.is_empty());
    assert_no_sentinel(&sink);
}

#[test]
fn fixture_folder_imports_like_the_zip() {
    let dir = tempfile::tempdir().unwrap();
    let (_, from_zip) = run(&imp(), &fixture_zip(dir.path()));
    let (_, from_dir) = run(&imp(), &fixture("x/archive"));
    assert_eq!(from_zip.interactions, from_dir.interactions);
}

#[test]
fn committed_fixture_zip_matches_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (_, from_folder) = run(&imp(), &fixture("x/archive"));
    let (_, committed) = run(&imp(), &fixture("x/twitter-archive.zip"));
    assert_eq!(committed.interactions, from_folder.interactions);
    let _ = dir;
}
