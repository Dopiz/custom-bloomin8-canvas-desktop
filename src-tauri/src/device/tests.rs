use std::time::Duration;

use serde_json::json;

use super::client::DeviceClient;
use super::error::DeviceError;
use super::mock::MockDevice;
use super::types::{DeviceSettingsUpdate, ShowRequest};

#[tokio::test]
async fn info_parses_device_fields() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    let info = client.info().await.expect("info should succeed");

    assert_eq!(info.width, 1200);
    assert_eq!(info.height, 1600);
    assert_eq!(info.battery, Some(80));
    assert_eq!(info.gallery.as_deref(), Some("default"));
    assert_eq!(info.max_idle, Some(120));
}

#[tokio::test]
async fn wait_ready_succeeds_once_state_reaches_ready() {
    let mock = MockDevice::start().await;
    mock.set_status(50, "Processing");
    mock.set_status_after(Duration::from_millis(60), 100, "Ready");

    let client = DeviceClient::new(mock.base_url());
    let state = client
        .wait_ready(Duration::from_secs(2), Duration::from_millis(20))
        .await
        .expect("device should become ready");

    assert_eq!(state.status, 100);
    assert_eq!(state.msg, "Ready");
}

#[tokio::test]
async fn wait_ready_times_out_when_device_never_becomes_ready() {
    let mock = MockDevice::start().await;
    mock.set_status(50, "Processing");

    let client = DeviceClient::new(mock.base_url());
    let err = client
        .wait_ready(Duration::from_millis(120), Duration::from_millis(20))
        .await
        .expect_err("should time out, device never reaches Ready");

    match err {
        DeviceError::NotReady(timeout, last) => {
            assert_eq!(timeout, Duration::from_millis(120));
            let last = last.expect("should have observed at least one non-ready state");
            assert_eq!(last.status, 50);
        }
        other => panic!("expected NotReady, got {other:?}"),
    }
}

#[tokio::test]
async fn whistle_power_and_settings_calls_reach_the_mock() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    client.whistle().await.expect("whistle should succeed");
    client.reboot().await.expect("reboot should succeed");
    client.clear_screen().await.expect("clear_screen should succeed");
    client
        .set_settings(&DeviceSettingsUpdate {
            max_idle: Some(300),
            sleep_duration: Some(3600),
            ..Default::default()
        })
        .await
        .expect("set_settings should succeed");

    assert_eq!(mock.hit_count("whistle"), 1);
    assert_eq!(mock.hit_count("reboot"), 1);
    assert_eq!(mock.hit_count("clearScreen"), 1);
    assert_eq!(mock.hit_count("settings"), 1);
}

#[tokio::test]
async fn sleep_call_reaches_the_mock_and_puts_it_to_sleep() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    client.sleep().await.expect("sleep should succeed");

    assert_eq!(mock.hit_count("sleep"), 1);
    assert!(mock.is_asleep());
}

#[tokio::test]
async fn info_fails_within_timeout_when_device_is_asleep() {
    let mock = MockDevice::start().await;
    mock.set_asleep(true);

    let client = DeviceClient::new(mock.base_url()).with_timeouts(
        Duration::from_secs(3),
        Duration::from_secs(5),
        Duration::from_secs(15),
    );

    let started = std::time::Instant::now();
    let result = client.info().await;
    let elapsed = started.elapsed();

    assert!(result.is_err(), "info() should fail while device is asleep");
    assert!(
        elapsed < Duration::from_millis(3_500),
        "info() should fail within ~3s of its timeout, took {elapsed:?}"
    );
}

/// Exercises the MockDevice's HTTP surface directly (DeviceClient does not
/// implement upload/delete in this slice) to prove the mock
/// reproduces the firmware's filename-cache bug: re-uploading a filename,
/// even after deleting it, keeps serving the FIRST upload's content, while
/// `/state` and `deviceInfo.image` both look like the refresh succeeded.
#[tokio::test]
async fn mock_reproduces_filename_cache_bug() {
    let mock = MockDevice::start().await;
    let http = reqwest::Client::new();
    let base = mock.base_url();

    let upload = |body: &'static [u8]| {
        let http = http.clone();
        let base = base.clone();
        async move {
            http.post(format!("{base}/upload"))
                .query(&[("filename", "frame.jpg"), ("gallery", "default"), ("show_now", "1")])
                .multipart(reqwest::multipart::Form::new().part(
                    "image",
                    reqwest::multipart::Part::bytes(body.to_vec()),
                ))
                .send()
                .await
                .expect("upload request should succeed")
        }
    };

    let resp = upload(b"AAAA-first-upload").await;
    assert!(resp.status().is_success());

    let resp = http
        .post(format!("{base}/image/delete"))
        .query(&[("image", "frame.jpg"), ("gallery", "default")])
        .send()
        .await
        .expect("delete request should succeed");
    assert!(resp.status().is_success());

    let resp = upload(b"BBBB-second-upload-different-content").await;
    assert!(resp.status().is_success());

    let client = DeviceClient::new(mock.base_url());

    // /state still reports Ready...
    let state = client.state().await.expect("state should succeed");
    assert_eq!(state.status, 100);

    // ...and deviceInfo.image points at the (re-)uploaded filename...
    let info = client.info().await.expect("info should succeed");
    assert!(
        info.image.as_deref().unwrap_or_default().ends_with("frame.jpg"),
        "expected image to end with frame.jpg, got {:?}",
        info.image
    );

    // ...but the content actually cached under that filename is still the
    // FIRST upload's bytes — the panel would still be showing stale content.
    assert_eq!(
        mock.stored_content("default", "frame.jpg"),
        Some(b"AAAA-first-upload".to_vec())
    );
}

// ---------------------------------------------------------------------
// upload_and_show + guarded cleanup
// ---------------------------------------------------------------------

#[tokio::test]
async fn upload_and_show_succeeds_and_info_reflects_new_filename() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    let result = client
        .upload_and_show(b"first-frame".to_vec(), "frame", "default", "20260101_000000")
        .await
        .expect("upload_and_show should succeed");

    assert_eq!(result.filename, "frame_20260101_000000.jpg");
    assert_eq!(result.gallery, "default");

    let info = client.info().await.expect("info should succeed");
    assert!(
        info.image.as_deref().unwrap_or_default().ends_with(&result.filename),
        "expected deviceInfo.image to end with {:?}, got {:?}",
        result.filename,
        info.image
    );
}

#[tokio::test]
async fn upload_and_show_fails_verification_when_device_does_not_update_image() {
    let mock = MockDevice::start().await;
    mock.suppress_next_image_update();
    let client = DeviceClient::new(mock.base_url());

    let err = client
        .upload_and_show(b"frame-bytes".to_vec(), "frame", "default", "20260101_000000")
        .await
        .expect_err("upload_and_show should fail when the device never updates its image");

    match err {
        DeviceError::DisplayVerificationFailed { expected, actual } => {
            assert_eq!(expected, "frame_20260101_000000.jpg");
            assert!(actual.is_none(), "mock never displayed anything, expected None, got {actual:?}");
        }
        other => panic!("expected DisplayVerificationFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn cleanup_rejects_empty_or_missing_keep_without_touching_the_gallery() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    client
        .upload(b"a".to_vec(), "frame_1.jpg", "default", false)
        .await
        .expect("seed upload should succeed");

    let err = client
        .cleanup("frame_", Some(""), "default")
        .await
        .expect_err("cleanup should refuse an empty keep");
    assert!(matches!(err, DeviceError::CleanupRefused));

    let err = client
        .cleanup("frame_", None, "default")
        .await
        .expect_err("cleanup should refuse a missing keep");
    assert!(matches!(err, DeviceError::CleanupRefused));

    assert_eq!(mock.gallery_file_names("default"), vec!["frame_1.jpg".to_string()]);
}

#[tokio::test]
async fn cleanup_deletes_prefix_matches_except_keep() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    for name in ["frame_1.jpg", "frame_2.jpg", "frame_3.jpg", "other_1.jpg"] {
        client
            .upload(b"x".to_vec(), name, "default", false)
            .await
            .expect("seed upload should succeed");
    }

    let deleted = client
        .cleanup("frame_", Some("frame_3.jpg"), "default")
        .await
        .expect("cleanup should succeed");

    let mut deleted_sorted = deleted;
    deleted_sorted.sort();
    assert_eq!(deleted_sorted, vec!["frame_1.jpg".to_string(), "frame_2.jpg".to_string()]);

    let mut remaining = mock.gallery_file_names("default");
    remaining.sort();
    assert_eq!(remaining, vec!["frame_3.jpg".to_string(), "other_1.jpg".to_string()]);

    let listed = client
        .gallery_images("default", 0, 200)
        .await
        .expect("gallery_images should succeed");
    let mut listed_names: Vec<String> = listed.into_iter().map(|i| i.name).collect();
    listed_names.sort();
    assert_eq!(listed_names, vec!["frame_3.jpg".to_string(), "other_1.jpg".to_string()]);
}

// ---------------------------------------------------------------------
// BLE wake pulse + wake_if_needed
// ---------------------------------------------------------------------

#[tokio::test]
async fn wake_if_needed_returns_immediately_when_already_awake() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    let waker_calls = std::sync::atomic::AtomicU32::new(0);
    client
        .wake_if_needed_with(
            |_name| {
                waker_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async {}
            },
            "Bloomin8".to_string(),
            Duration::from_millis(10),
            Duration::from_millis(200),
        )
        .await
        .expect("device is already awake, should succeed without waking");

    assert_eq!(waker_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn wake_if_needed_invokes_stub_waker_and_succeeds_once_mock_wakes() {
    let mock = MockDevice::start().await;
    mock.set_asleep(true);
    let client = DeviceClient::new(mock.base_url()).with_timeouts(
        Duration::from_millis(50),
        Duration::from_secs(5),
        Duration::from_secs(15),
    );

    // Stub waker: instead of a real BLE pulse, just flip the mock's sleep
    // gate — mirroring how the real pulse would eventually let /deviceInfo
    // start responding again.
    client
        .wake_if_needed_with(
            |_name| {
                mock.set_asleep(false);
                async {}
            },
            "Bloomin8".to_string(),
            Duration::from_millis(20),
            Duration::from_secs(2),
        )
        .await
        .expect("wake_if_needed should succeed once the stub wake pulse wakes the mock");
}

#[tokio::test]
async fn wake_if_needed_times_out_when_mock_never_wakes() {
    let mock = MockDevice::start().await;
    mock.set_asleep(true);
    let client = DeviceClient::new(mock.base_url()).with_timeouts(
        Duration::from_millis(50),
        Duration::from_secs(5),
        Duration::from_secs(15),
    );

    let started = std::time::Instant::now();
    let err = client
        .wake_if_needed_with(
            |_name| async {
                // Stub pulse that never actually wakes the mock.
            },
            "Bloomin8".to_string(),
            Duration::from_millis(20),
            Duration::from_millis(150),
        )
        .await
        .expect_err("wake_if_needed should time out when the device never wakes");

    match err {
        DeviceError::WakeTimeout(budget) => assert_eq!(budget, Duration::from_millis(150)),
        other => panic!("expected WakeTimeout, got {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "test should use the injected short budget, not the real 45s default"
    );
}

// ---------------------------------------------------------------------
// Gallery/Playlist client API
// ---------------------------------------------------------------------

#[tokio::test]
async fn gallery_crud_round_trips() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    client
        .gallery_create("vacation")
        .await
        .expect("gallery_create should succeed");

    let names: Vec<String> = client
        .gallery_list()
        .await
        .expect("gallery_list should succeed")
        .into_iter()
        .map(|g| g.name)
        .collect();
    assert!(names.contains(&"vacation".to_string()));
    // The mock starts with a "default" gallery already present.
    assert!(names.contains(&"default".to_string()));

    client
        .gallery_delete("vacation")
        .await
        .expect("gallery_delete should succeed");

    let names_after: Vec<String> = client
        .gallery_list()
        .await
        .expect("gallery_list should succeed")
        .into_iter()
        .map(|g| g.name)
        .collect();
    assert!(!names_after.contains(&"vacation".to_string()));
}

#[tokio::test]
async fn gallery_images_pagination_returns_expected_slices() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    for i in 0..5 {
        client
            .upload(b"x".to_vec(), &format!("img_{i}.jpg"), "default", false)
            .await
            .expect("seed upload should succeed");
    }

    let page1 = client
        .gallery_images("default", 0, 2)
        .await
        .expect("gallery_images should succeed");
    assert_eq!(
        page1.iter().map(|i| i.name.clone()).collect::<Vec<_>>(),
        vec!["img_0.jpg".to_string(), "img_1.jpg".to_string()]
    );

    let page2 = client
        .gallery_images("default", 2, 2)
        .await
        .expect("gallery_images should succeed");
    assert_eq!(
        page2.iter().map(|i| i.name.clone()).collect::<Vec<_>>(),
        vec!["img_2.jpg".to_string(), "img_3.jpg".to_string()]
    );

    let page3 = client
        .gallery_images("default", 4, 2)
        .await
        .expect("gallery_images should succeed");
    assert_eq!(
        page3.iter().map(|i| i.name.clone()).collect::<Vec<_>>(),
        vec!["img_4.jpg".to_string()]
    );
}

#[tokio::test]
async fn show_changes_mock_reported_device_info() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    client
        .show(&ShowRequest::Image {
            image: "/gallerys/default/f1.jpg".to_string(),
        })
        .await
        .expect("show(Image) should succeed");
    let info = client.info().await.expect("info should succeed");
    assert_eq!(info.image.as_deref(), Some("/gallerys/default/f1.jpg"));
    assert_eq!(
        info.extra.get("now_playing").and_then(|v| v.as_str()),
        Some("image:/gallerys/default/f1.jpg")
    );

    client
        .show(&ShowRequest::Gallery {
            gallery: "vacation".to_string(),
            duration: 30,
        })
        .await
        .expect("show(Gallery) should succeed");
    let info = client.info().await.expect("info should succeed");
    assert_eq!(info.gallery.as_deref(), Some("vacation"));
    assert_eq!(
        info.extra.get("now_playing").and_then(|v| v.as_str()),
        Some("gallery:vacation")
    );

    client
        .show(&ShowRequest::Playlist {
            playlist: "morning".to_string(),
        })
        .await
        .expect("show(Playlist) should succeed");
    let info = client.info().await.expect("info should succeed");
    assert_eq!(
        info.extra.get("now_playing").and_then(|v| v.as_str()),
        Some("playlist:morning")
    );

    client.show_next().await.expect("show_next should succeed");
    assert_eq!(mock.hit_count("showNext"), 1);
}

#[tokio::test]
async fn playlist_crud_round_trips() {
    let mock = MockDevice::start().await;
    let client = DeviceClient::new(mock.base_url());

    client
        .playlist_put(
            "morning",
            json!({
                "type": "duration",
                "list": [{"name": "f1.jpg", "duration": 40, "time": ""}],
            }),
        )
        .await
        .expect("playlist_put should succeed");

    let names: Vec<String> = client
        .playlist_list()
        .await
        .expect("playlist_list should succeed")
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert!(names.contains(&"morning".to_string()));

    let fetched = client
        .playlist_get("morning")
        .await
        .expect("playlist_get should succeed");
    assert_eq!(fetched.get("name").and_then(|v| v.as_str()), Some("morning"));
    assert_eq!(fetched.get("type").and_then(|v| v.as_str()), Some("duration"));

    client
        .playlist_delete("morning")
        .await
        .expect("playlist_delete should succeed");

    let names_after: Vec<String> = client
        .playlist_list()
        .await
        .expect("playlist_list should succeed")
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert!(!names_after.contains(&"morning".to_string()));
}
