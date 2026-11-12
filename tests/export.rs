//! `unigif export` and importing the archive into an empty instance.
#![allow(clippy::field_reassign_with_default)]
mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

#[tokio::test]
async fn an_export_restores_into_an_empty_instance() {
    let a = Harness::new().await;
    let cat = a.add_gif(1, "cat,funny").await;
    let cat_id = cat["id"].as_str().unwrap().to_string();
    a.call(
        "PATCH",
        &format!("/api/v1/gifs/{cat_id}"),
        Some(json!({"description": "A cat falls off a desk"})),
    )
    .await;
    for _ in 0..3 {
        a.call("POST", &format!("/api/v1/gifs/{cat_id}/use"), None).await;
    }
    let unsorted = a.add_gif(2, "").await;
    // A transparent GIF keeps its original, which is what gets exported.
    let path = a.dir.path().join("alpha.gif");
    transparent_gif_fixture(&path);
    let (s, alpha) = a.add("?wait=60&tags=sticker", std::fs::read(&path).unwrap()).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(alpha["original_kept"], true);
    // Trashed GIFs stay behind.
    let gone = a.add_gif(3, "trashed").await;
    a.call("DELETE", &format!("/api/v1/gifs/{}", gone["id"].as_str().unwrap()), None).await;

    let zip = a.dir.path().join("library.zip");
    let summary = unigif::export::export(&a.app, &zip).await.unwrap();
    assert_eq!(summary.gifs, 3);
    assert!(zip.exists() && !zip.with_extension("zip.part").exists());
    assert!(unigif::export::is_export(&zip));

    // The archive: a manifest and one file per GIF, the original for the transparent one.
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
    let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
    names.sort();
    let alpha_id = alpha["id"].as_str().unwrap();
    assert!(names.contains(&"manifest.json".to_string()));
    assert!(names.contains(&format!("media/{alpha_id}.gif")), "{names:?}");
    assert!(names.contains(&format!("media/{cat_id}.mp4")), "{names:?}");
    assert_eq!(names.len(), 4, "{names:?}");
    let original = std::fs::read(&path).unwrap();
    let mut stored = Vec::new();
    std::io::Read::read_to_end(&mut archive.by_name(&format!("media/{alpha_id}.gif")).unwrap(), &mut stored)
        .unwrap();
    assert_eq!(stored, original, "the original is exported byte for byte");

    // Import into a fresh instance.
    let b = Harness::new().await;
    let opts = unigif::import::Options { dir: zip.clone(), tags_from_folders: false };
    let r = unigif::import::run(&b.app, &opts).await.unwrap();
    assert_eq!((r.added, r.duplicates, r.rejected, r.failed), (3, 0, 0, 0));

    let (_, page) = b.call("GET", "/api/v1/gifs?sort=old", None).await;
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    let by_tag =
        |t: &str| items.iter().find(|g| g["tags"].as_array().unwrap().iter().any(|x| x == t)).unwrap();
    let cat_b = by_tag("cat");
    let mut tags: Vec<&str> = cat_b["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
    tags.sort();
    assert_eq!(tags, ["cat", "funny"]);
    assert_eq!(cat_b["description"], "A cat falls off a desk");
    assert_eq!(cat_b["created_at"], cat["created_at"], "the date added survives");
    assert_eq!(cat_b["status"], "ready");
    // Usage survives: it sorts first by use.
    let (_, used) = b.call("GET", "/api/v1/gifs?sort=used&limit=1", None).await;
    assert_eq!(used["items"][0]["id"], cat_b["id"]);
    // The transparent one is transparent again, from its original.
    let alpha_b = by_tag("sticker");
    assert_eq!((alpha_b["has_alpha"].clone(), alpha_b["original_kept"].clone()), (json!(true), json!(true)));
    // The unsorted one is still unsorted.
    let (_, u) = b.call("GET", "/api/v1/gifs?unsorted=true", None).await;
    assert_eq!(u["items"].as_array().unwrap().len(), 1);
    assert_eq!(u["items"][0]["created_at"], unsorted["created_at"]);
    // Nothing from the trash came along.
    let (_, t) = b.call("GET", "/api/v1/tags", None).await;
    assert!(!t.to_string().contains("trashed"));

    // Importing the same archive again only finds duplicates.
    let r = unigif::import::run(&b.app, &opts).await.unwrap();
    assert_eq!((r.added, r.duplicates), (0, 3));
}

#[tokio::test]
async fn a_zip_that_is_not_an_export_is_refused() {
    let h = Harness::new().await;
    let path = h.dir.path().join("other.zip");
    let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    z.start_file("a.txt", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut z, b"hello").unwrap();
    z.finish().unwrap();
    let opts = unigif::import::Options { dir: path, tags_from_folders: false };
    let err = unigif::import::run(&h.app, &opts).await.unwrap_err();
    assert!(err.to_string().contains("not a unigif export"), "{err}");
}
