//! `PathPolicy`: confinement to read/write roots, `..` and symlink escapes,
//! and its enforcement by the commands.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::path::Path;

use common::{Runner, mini_pbip, tmp_dir};
use pbi_ops::{OpErrorCode, PathPolicy, Registry, Workspace};
use serde_json::json;

fn denied(r: Result<std::path::PathBuf, pbi_ops::OpError>) -> bool {
    matches!(r, Err(e) if e.code == OpErrorCode::PathDenied)
}

#[test]
fn unrestricted_paths_pass_through() {
    let p = PathPolicy::unrestricted();
    assert!(!p.is_restricted());
    assert_eq!(
        p.check_read(Path::new("../x.pbip")).unwrap(),
        Path::new("../x.pbip")
    );
    assert_eq!(
        p.check_write(Path::new("/tmp/out.png")).unwrap(),
        Path::new("/tmp/out.png")
    );
    assert_eq!(
        p.check_read(Path::new("")).unwrap_err().code,
        OpErrorCode::InvalidParams
    );
}

#[test]
fn reads_and_writes_stay_inside_their_roots() {
    let dir = tmp_dir("pbi-ops-policy-");
    let read = dir.path().join("in");
    let write = dir.path().join("out");
    let outside = dir.path().join("secret");
    for d in [&read, &write, &outside] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(read.join("model.pbip"), "{}").unwrap();
    fs::write(outside.join("data.csv"), "a").unwrap();
    let p = PathPolicy::restricted([&read], [&write]).unwrap();
    assert!(p.is_restricted());

    // Inside: absolute, relative (to the first read root), missing files.
    let ok = p.check_read(&read.join("model.pbip")).unwrap();
    assert!(ok.starts_with(fs::canonicalize(&read).unwrap()));
    assert_eq!(p.check_read(Path::new("model.pbip")).unwrap(), ok);
    assert!(p.check_read(&read.join("sub/missing.csv")).is_ok());
    assert!(p.check_write(&write.join("a/b/c.png")).is_ok());
    // Write roots are readable too.
    assert!(p.check_read(&write.join("x.csv")).is_ok());

    // Escapes.
    assert!(denied(p.check_read(&outside.join("data.csv"))));
    assert!(denied(p.check_read(&read.join("../secret/data.csv"))));
    assert!(denied(p.check_read(Path::new("../secret/data.csv"))));
    assert!(denied(
        p.check_read(&read.join("missing/../../secret/data.csv"))
    ));
    assert!(
        denied(p.check_write(&read.join("x.png"))),
        "read root is not writable"
    );
    assert!(denied(p.check_write(&outside.join("x.png"))));
    assert!(denied(p.check_write(&write.join("../secret/x.png"))));
    assert!(denied(p.check_write(&write.join("new/../../secret/x.png"))));

    // Roots must exist.
    assert_eq!(
        PathPolicy::restricted([dir.path().join("nope")], Vec::<&Path>::new())
            .unwrap_err()
            .code,
        OpErrorCode::NotFound
    );
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_escape() {
    use std::os::unix::fs::symlink;
    let dir = tmp_dir("pbi-ops-symlink-");
    let root = dir.path().join("root");
    let outside = dir.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.csv"), "a").unwrap();
    symlink(outside.join("secret.csv"), root.join("link.csv")).unwrap();
    symlink(&outside, root.join("dirlink")).unwrap();
    symlink(dir.path().join("nowhere"), root.join("dangling")).unwrap();
    fs::write(root.join("inside.csv"), "b").unwrap();
    symlink(root.join("inside.csv"), root.join("inner-link.csv")).unwrap();
    let p = PathPolicy::restricted([&root], [&root]).unwrap();

    assert!(denied(p.check_read(&root.join("link.csv"))));
    assert!(denied(p.check_read(&root.join("dirlink/secret.csv"))));
    assert!(denied(p.check_write(&root.join("dirlink/new.png"))));
    assert!(denied(p.check_write(&root.join("dangling"))));
    assert!(denied(p.check_read(&root.join("dangling"))));
    // A link that stays inside is readable, but never written through.
    assert!(p.check_read(&root.join("inner-link.csv")).is_ok());
    assert!(denied(p.check_write(&root.join("inner-link.csv"))));
}

#[test]
fn blob_maps_may_not_point_outside() {
    let dir = tmp_dir("pbi-ops-blobmap-");
    let root = dir.path().join("root");
    fs::create_dir_all(&root).unwrap();
    fs::write(dir.path().join("secret.csv"), "a").unwrap();
    fs::write(
        root.join("map.json"),
        r#"{"containers": {"https://x.blob.core.windows.net/c": {"a.csv": "../secret.csv"}}}"#,
    )
    .unwrap();
    fs::write(
        root.join("ok.json"),
        r#"{"containers": {"https://x.blob.core.windows.net/c": {"a.csv": "data/a.csv"}}}"#,
    )
    .unwrap();
    let p = PathPolicy::restricted([&root], Vec::<&Path>::new()).unwrap();
    assert!(denied(p.check_blob_map(&root.join("map.json"))));
    assert!(p.check_blob_map(&root.join("ok.json")).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_apply_the_policy() {
    let dir = tmp_dir("pbi-ops-policy-cmd-");
    let read = dir.path().join("in");
    let write = dir.path().join("out");
    fs::create_dir_all(&read).unwrap();
    fs::create_dir_all(&write).unwrap();
    let pbip = mini_pbip(&read);
    let other = tmp_dir("pbi-ops-policy-other-");
    let outside = mini_pbip(other.path());
    let policy = PathPolicy::restricted([&read], [&write]).unwrap();
    let r = Runner::with(Registry::default_catalog(), Workspace::with_policy(policy));

    let e = r
        .try_run("doc.open", json!({"path": outside}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::PathDenied);
    let e = r
        .try_run("doc.detect", json!({"path": "../../etc/passwd"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::PathDenied);

    let doc = r
        .open(json!({"path": "Mini.pbip", "autoRefresh": false}))
        .await;
    let e = r
        .try_run(
            "export.model",
            json!({"docId": doc, "format": "json", "path": read.join("model.json")}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        e.code,
        OpErrorCode::PathDenied,
        "read roots are not writable"
    );
    let e = r
        .try_run(
            "convert.pbip",
            json!({"docId": doc, "outDir": dir.path().join("elsewhere")}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::PathDenied);
    let out = r
        .run(
            "export.model",
            json!({"docId": doc, "format": "markdown", "path": write.join("model.md")}),
        )
        .await;
    assert!(write.join("model.md").exists(), "{out}");
    let _ = pbip;
}
