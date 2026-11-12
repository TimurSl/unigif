#![allow(clippy::field_reassign_with_default)]
mod common;

use axum::http::StatusCode;
use common::*;

/// A red square on a genuinely transparent background. Only a palette with a
/// reserved transparent entry yields a GIF that really has alpha.
fn transparent_gif_fixture(path: &std::path::Path) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=black@0:s=80x80:r=10:d=1,format=rgba",
        "-f",
        "lavfi",
        "-i",
        "color=c=red@1:s=40x40:r=10:d=1,format=rgba",
        "-filter_complex",
        "[0:v][1:v]overlay=20:20:format=auto,format=rgba,split[a][b];\
         [a]palettegen=reserve_transparent=1[p];[b][p]paletteuse",
        path.to_str().unwrap(),
    ]);
}

#[tokio::test]
async fn quick_add_gif_produces_all_stored_variants() {
    // Default encoder preset here, so the size comparison below is realistic.
    let h = Harness::with(|c| c.media.preset = "slow".into()).await;
    let src = gif(h.dir.path(), "a.gif", "320x240", 1);

    let (status, v) = h.add("?tags=Cat,%23funny&wait=30", src.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "ready", "{v}");
    assert_eq!(v["tags"], serde_json::json!(["cat", "funny"]));
    assert_eq!((v["width"].as_i64(), v["height"].as_i64()), (Some(320), Some(240)));
    assert_eq!(v["has_alpha"], false);
    assert!(v["thumbhash"].as_str().is_some_and(|t| !t.is_empty()));
    let id = v["id"].as_str().unwrap();

    // full.mp4 is H.264 / yuv420p / silent, and smaller than the GIF.
    let full = media_path(&h, id, "full.mp4");
    let p = probe(&full);
    let vs = video_stream(&p);
    assert_eq!(vs["codec_name"], "h264");
    assert_eq!(vs["pix_fmt"], "yuv420p");
    assert!(p["streams"].as_array().unwrap().iter().all(|s| s["codec_type"] != "audio"));
    assert!(std::fs::metadata(&full).unwrap().len() < src.len() as u64);

    // preview is at most 200 px tall; poster is a decodable webp.
    let pv = probe(&media_path(&h, id, "preview.mp4"));
    assert!(video_stream(&pv)["height"].as_u64().unwrap() <= 200);
    let poster = std::fs::read(media_path(&h, id, "poster.webp")).unwrap();
    assert_eq!(&poster[..4], b"RIFF");
    assert_eq!(&poster[8..12], b"WEBP");

    // Opaque GIF: original discarded under the default policy.
    assert!(!media_path(&h, id, "original.gif").exists());
    assert_eq!(h.app.cfg.work_dir().read_dir().unwrap().count(), 0, "source left in work/");

    // Every URL in the object resolves.
    for key in ["mp4", "preview_mp4", "poster"] {
        let url = v["media"][key]["url"].as_str().unwrap();
        let resp = h.send(h.get(url)).await;
        assert_eq!(resp.status(), StatusCode::OK, "{url}");
    }
}

#[tokio::test]
async fn same_bytes_twice_is_a_duplicate_and_merges_tags() {
    let h = Harness::new().await;
    let src = gif(h.dir.path(), "a.gif", "160x120", 2);

    let (s1, v1) = h.add("?tags=a&wait=30", src.clone()).await;
    assert_eq!(s1, StatusCode::CREATED);
    let (s2, v2) = h.add("?tags=b", src).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(v2["duplicate"], true);
    assert_eq!(v2["id"], v1["id"]);
    assert_eq!(v2["tags"], serde_json::json!(["a", "b"]));
}

#[tokio::test]
async fn source_id_makes_adds_idempotent() {
    let h = Harness::new().await;
    let a = gif(h.dir.path(), "a.gif", "160x120", 3);
    let b = gif(h.dir.path(), "b.gif", "160x120", 4); // different bytes

    let (s1, v1) = h.add("?source=telegram&source_id=5368&wait=30", a).await;
    assert_eq!(s1, StatusCode::CREATED);
    let (s2, v2) = h.add("?source=telegram&source_id=5368", b).await;
    assert_eq!(s2, StatusCode::OK, "{v2}");
    assert_eq!(v2["duplicate"], true);
    assert_eq!(v2["id"], v1["id"]);
}

#[tokio::test]
async fn h264_without_audio_is_remuxed_not_reencoded() {
    let h = Harness::new().await;
    let mp4 = h.dir.path().join("tg.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x240:rate=15:duration=1.5",
        "-an",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-preset",
        "ultrafast",
        mp4.to_str().unwrap(),
    ]);
    let src = std::fs::read(&mp4).unwrap();
    let (status, v) = h.add("?wait=30", src.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "ready", "{v}");
    let id = v["id"].as_str().unwrap();

    // A remux keeps the very same packets, so the video bitstream size matches.
    let src_p = probe(&mp4);
    let out_p = probe(&media_path(&h, id, "full.mp4"));
    assert_eq!(video_stream(&src_p)["nb_frames"], video_stream(&out_p)["nb_frames"]);
    assert_eq!(video_stream(&src_p)["width"], video_stream(&out_p)["width"]);
}

#[tokio::test]
async fn mp4_with_audio_is_stripped_and_large_video_is_downscaled() {
    let h = Harness::new().await;
    let mp4 = h.dir.path().join("big.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=1280x720:rate=10:duration=1",
        "-f",
        "lavfi",
        "-i",
        "sine=duration=1",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-preset",
        "ultrafast",
        "-c:a",
        "aac",
        "-shortest",
        mp4.to_str().unwrap(),
    ]);
    let (status, v) = h.add("?wait=60", std::fs::read(&mp4).unwrap()).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "ready", "{v}");
    let id = v["id"].as_str().unwrap();
    let p = probe(&media_path(&h, id, "full.mp4"));
    assert!(p["streams"].as_array().unwrap().iter().all(|s| s["codec_type"] != "audio"));
    assert_eq!(video_stream(&p)["width"], 720);
    assert_eq!(video_stream(&p)["height"], 404);
}

#[tokio::test]
async fn odd_dimensions_become_even() {
    let h = Harness::new().await;
    let src = gif(h.dir.path(), "odd.gif", "161x121", 5);
    let (status, v) = h.add("?wait=30", src).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "ready", "{v}");
    assert_eq!(v["width"].as_i64().unwrap() % 2, 0);
    assert_eq!(v["height"].as_i64().unwrap() % 2, 0);
}

#[tokio::test]
async fn single_frame_gif_works() {
    let h = Harness::new().await;
    let path = h.dir.path().join("one.gif");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=64x64:rate=1:duration=1",
        "-frames:v",
        "1",
        path.to_str().unwrap(),
    ]);
    let (status, v) = h.add("?wait=30", std::fs::read(path).unwrap()).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "ready", "{v}");
}

#[tokio::test]
async fn transparent_gif_keeps_original_and_composites_on_matte() {
    let h = Harness::new().await;
    // A red square on a transparent background.
    let path = h.dir.path().join("alpha.gif");
    transparent_gif_fixture(&path);
    let src = std::fs::read(&path).unwrap();
    let (status, v) = h.add("?wait=60", src.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "ready", "{v}");
    assert_eq!(v["has_alpha"], true, "{v}");
    let id = v["id"].as_str().unwrap();
    let kept = media_path(&h, id, "original.gif");
    assert_eq!(std::fs::read(kept).unwrap(), src, "original must be byte-identical");

    // Corner pixel (fully transparent in the source) is the matte colour: black.
    let png = h.dir.path().join("corner.png");
    ffmpeg(&[
        "-i",
        media_path(&h, id, "full.mp4").to_str().unwrap(),
        "-frames:v",
        "1",
        "-vf",
        "format=gray,crop=1:1:0:0",
        png.to_str().unwrap(),
    ]);
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i", png.to_str().unwrap(), "-f", "rawvideo", "-pix_fmt", "gray", "-"])
        .output()
        .unwrap();
    assert!(out.stdout[0] < 24, "corner should be near black, got {}", out.stdout[0]);
}

#[tokio::test]
async fn keep_originals_policy_is_honoured() {
    use unigif::config::KeepOriginals;
    let h = Harness::with(|c| c.media.keep_originals = KeepOriginals::Always).await;
    let src = gif(h.dir.path(), "a.gif", "160x120", 6);
    let (_, v) = h.add("?wait=30", src.clone()).await;
    let id = v["id"].as_str().unwrap();
    assert_eq!(std::fs::read(media_path(&h, id, "original.gif")).unwrap(), src);

    let h = Harness::with(|c| c.media.keep_originals = KeepOriginals::Never).await;
    let alpha = h.dir.path().join("alpha.gif");
    transparent_gif_fixture(&alpha);
    let (_, v) = h.add("?wait=60", std::fs::read(alpha).unwrap()).await;
    assert_eq!(v["has_alpha"], true);
    assert!(!media_path(&h, v["id"].as_str().unwrap(), "original.gif").exists());
}

#[tokio::test]
async fn rejects_bad_input_with_the_right_status() {
    let h = Harness::new().await;

    // Not media at all.
    let (s, v) = h.add("", b"<html>hello</html>".to_vec()).await;
    assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{v}");
    assert_eq!(v["code"], "unsupported_media");

    // Looks like a GIF but is truncated garbage.
    let (s, v) = h.add("", b"GIF89a\x01\x00\x01\x00\x80\x00\x00garbage".to_vec()).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "invalid_media");

    // Truncated real GIF.
    let src = gif(h.dir.path(), "a.gif", "160x120", 7);
    let (s, _) = h.add("", src[..src.len() / 2].to_vec()).await;
    assert!(s == StatusCode::UNPROCESSABLE_ENTITY || s == StatusCode::CREATED, "got {s}");

    // Empty body.
    let (s, _) = h.add("", vec![]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Bad tag.
    let src = gif(h.dir.path(), "b.gif", "160x120", 8);
    let (s, v) = h.add("?tags=two%20words%2Cx-y", src).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn limits_are_enforced_before_decoding() {
    let h = Harness::with(|c| {
        c.media.max_width = 100;
        c.media.max_frames = 5;
        c.upload.max_size = unigif::config::ByteSize(2000);
    })
    .await;
    let big = gif(h.dir.path(), "wide.gif", "200x100", 1);
    let (s, v) = h.add("", big).await;
    // Either over the 2000-byte cap or over the dimension limit; both refuse.
    assert!(s == StatusCode::PAYLOAD_TOO_LARGE || s == StatusCode::UNPROCESSABLE_ENTITY, "{s} {v}");

    let h = Harness::with(|c| {
        c.media.max_width = 100;
    })
    .await;
    let (s, v) = h.add("", gif(h.dir.path(), "wide.gif", "200x100", 1)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(v["detail"].as_str().unwrap().contains("dimensions"), "{v}");

    let h = Harness::with(|c| c.media.max_frames = 5).await;
    let (s, v) = h.add("", gif(h.dir.path(), "long.gif", "64x64", 1)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(v["detail"].as_str().unwrap().contains("frame"), "{v}");
}

#[tokio::test]
async fn upload_cap_is_enforced_while_streaming() {
    let h = Harness::with(|c| c.upload.max_size = unigif::config::ByteSize(1000)).await;
    let (s, v) = h.add("", vec![b'x'; 5000]).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
    assert_eq!(v["code"], "too_large");
    // No partial file left behind.
    assert_eq!(h.app.cfg.tmp_dir().read_dir().unwrap().count(), 0);
}

#[tokio::test]
async fn multipart_upload_with_fields() {
    let h = Harness::new().await;
    let src = gif(h.dir.path(), "a.gif", "160x120", 9);
    let boundary = "XBOUNDARYX";
    let mut body = Vec::new();
    let text = |name: &str, val: &str| {
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{val}\r\n")
    };
    body.extend(text("tags", "#a #b").into_bytes());
    body.extend(text("source", "share").into_bytes());
    body.extend(text("wait", "30").into_bytes());
    body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.gif\"\r\nContent-Type: image/gif\r\n\r\n").into_bytes());
    body.extend(&src);
    body.extend(format!("\r\n--{boundary}--\r\n").into_bytes());
    let resp =
        h.send(h.post("/api/v1/gifs", &format!("multipart/form-data; boundary={boundary}"), body)).await;
    let (s, v) = json(resp).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["tags"], serde_json::json!(["a", "b"]));
    assert_eq!(v["source"]["type"], "share");
    assert_eq!(v["status"], "ready", "{v}");
}

#[tokio::test]
async fn jobs_resume_after_a_restart() {
    // Add with wait=0 and no worker running, "restart", and see it finish.
    let dir = tempfile::tempdir().unwrap();
    let make = |dir: &std::path::Path| {
        let mut cfg = unigif::config::Config::default();
        cfg.data_dir = dir.join("data");
        cfg.cache_dir = dir.join("cache");
        cfg.auth.bootstrap_token = Some(TOKEN.into());
        cfg.media.preset = "ultrafast".into();
        cfg
    };
    let src = gif(dir.path(), "a.gif", "160x120", 10);

    let id = {
        let cfg = make(dir.path());
        for d in [cfg.media_dir(), cfg.tmp_dir(), cfg.work_dir()] {
            std::fs::create_dir_all(d).unwrap();
        }
        let pool = unigif::db::open(&cfg.db_path()).await.unwrap();
        let app = unigif::app::AppState::new(cfg, pool);
        // No workers: the job stays queued, as if the process died right after the insert.
        let tmp = app.cfg.tmp_dir().join("up");
        std::fs::write(&tmp, &src).unwrap();
        let up = unigif::ingest::receive(
            &app,
            tokio_util::io::ReaderStream::new(tokio::fs::File::open(&tmp).await.unwrap()),
        )
        .await
        .unwrap();
        let added = unigif::ingest::add(&app, up, unigif::ingest::AddRequest::new("api")).await.unwrap();
        assert_eq!(added.gif.status, "processing");
        sqlx::query("UPDATE jobs SET state = 'running'").execute(&app.db).await.unwrap(); // died mid-job
        added.gif.id
    };

    let cfg = make(dir.path());
    let pool = unigif::db::open(&cfg.db_path()).await.unwrap();
    let app = unigif::app::AppState::new(cfg, pool);
    unigif::ingest::worker::recover(&app).await.unwrap();
    unigif::ingest::worker::spawn(&app, 1);
    for _ in 0..300 {
        let row = unigif::db::gifs::get(&app.db, &id).await.unwrap().unwrap();
        if row.status == "ready" {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("job did not resume after restart");
}
