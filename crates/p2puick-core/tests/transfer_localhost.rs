use p2puick_core::{ProgressEvent, SessionConfig, TransferSession};
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::test]
async fn transfer_one_file_localhost() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("hello.txt");
    let dest_root = dir.path().join("out");
    std::fs::write(&src, b"hello p2puick").unwrap();
    std::fs::create_dir_all(&dest_root).unwrap();

    let code = "424242".to_string();
    let port = 47899u16;

    let host = TransferSession::new();
    let join = TransferSession::new();
    let (tx_h, mut rx_h) = mpsc::unbounded_channel::<ProgressEvent>();
    let (tx_j, mut rx_j) = mpsc::unbounded_channel::<ProgressEvent>();

    let host_task = tokio::spawn({
        let host = host.clone();
        let code = code.clone();
        async move {
            host.host_and_send(
                port,
                SessionConfig {
                    pairing_code: code,
                    hostname: "host".into(),
                    concurrency: 2,
                    exclude_dir_names: vec![],
                    retry_queue_path: None,
                },
                vec![src],
                tx_h,
            )
            .await
        }
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    let dest_for_join = dest_root.clone();
    let join_task = tokio::spawn({
        let join = join.clone();
        let code = code.clone();
        async move {
            join.join_and_receive(
                &format!("127.0.0.1:{port}"),
                SessionConfig {
                    pairing_code: code,
                    hostname: "joiner".into(),
                    concurrency: 2,
                    exclude_dir_names: vec![],
                    retry_queue_path: None,
                },
                dest_for_join,
                tx_j,
            )
            .await
        }
    });

    let (h, j) = tokio::join!(host_task, join_task);
    h.unwrap().unwrap();
    j.unwrap().unwrap();

    let received = std::fs::read(dest_root.join("hello.txt")).unwrap();
    assert_eq!(received, b"hello p2puick");

    // drain channels
    while rx_h.try_recv().is_ok() {}
    while rx_j.try_recv().is_ok() {}
}

#[tokio::test]
async fn second_transfer_skips_unchanged_file() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("hello.txt");
    let dest_root = dir.path().join("out");
    std::fs::write(&src, b"hello p2puick").unwrap();
    std::fs::create_dir_all(&dest_root).unwrap();

    async fn once(src: std::path::PathBuf, dest: std::path::PathBuf, port: u16, code: &str) {
        let host = TransferSession::new();
        let join = TransferSession::new();
        let (tx_h, mut rx_h) = mpsc::unbounded_channel::<ProgressEvent>();
        let (tx_j, mut rx_j) = mpsc::unbounded_channel::<ProgressEvent>();
        let host_task = tokio::spawn({
            let host = host.clone();
            let code = code.to_string();
            async move {
                host.host_and_send(
                    port,
                    SessionConfig {
                        pairing_code: code,
                        hostname: "host".into(),
                        concurrency: 2,
                        exclude_dir_names: vec![],
                        retry_queue_path: None,
                    },
                    vec![src],
                    tx_h,
                )
                .await
            }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        let join_task = tokio::spawn({
            let join = join.clone();
            let code = code.to_string();
            async move {
                join.join_and_receive(
                    &format!("127.0.0.1:{port}"),
                    SessionConfig {
                        pairing_code: code,
                        hostname: "joiner".into(),
                        concurrency: 2,
                        exclude_dir_names: vec![],
                        retry_queue_path: None,
                    },
                    dest,
                    tx_j,
                )
                .await
            }
        });
        let (h, j) = tokio::join!(host_task, join_task);
        h.unwrap().unwrap();
        j.unwrap().unwrap();
        let mut skipped = false;
        while let Ok(ev) = rx_h.try_recv() {
            if format!("{:?}", ev.kind).contains("FileSkipped")
                || ev.message.contains("Ignoré")
            {
                skipped = true;
            }
        }
        while let Ok(ev) = rx_j.try_recv() {
            if format!("{:?}", ev.kind).contains("FileSkipped")
                || ev.message.contains("Ignoré")
            {
                skipped = true;
            }
        }
        // First run won't skip; caller checks second run.
        let _ = skipped;
    }

    once(src.clone(), dest_root.clone(), 47901, "111111").await;
    once(src.clone(), dest_root.clone(), 47902, "222222").await;

    // Second pass should have skipped (mtime or hash).
    let host = TransferSession::new();
    let join = TransferSession::new();
    let (tx_h, mut rx_h) = mpsc::unbounded_channel::<ProgressEvent>();
    let (tx_j, mut rx_j) = mpsc::unbounded_channel::<ProgressEvent>();
    let port = 47903u16;
    let host_task = tokio::spawn({
        let host = host.clone();
        async move {
            host.host_and_send(
                port,
                SessionConfig {
                    pairing_code: "333333".into(),
                    hostname: "host".into(),
                    concurrency: 2,
                    exclude_dir_names: vec![],
                    retry_queue_path: None,
                },
                vec![src],
                tx_h,
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let join_task = tokio::spawn({
        let join = join.clone();
        let dest = dest_root.clone();
        async move {
            join.join_and_receive(
                &format!("127.0.0.1:{port}"),
                SessionConfig {
                    pairing_code: "333333".into(),
                    hostname: "joiner".into(),
                    concurrency: 2,
                    exclude_dir_names: vec![],
                    retry_queue_path: None,
                },
                dest,
                tx_j,
            )
            .await
        }
    });
    let (h, j) = tokio::join!(host_task, join_task);
    h.unwrap().unwrap();
    j.unwrap().unwrap();

    let mut skipped = false;
    while let Ok(ev) = rx_h.try_recv() {
        if ev.message.contains("Ignoré") {
            skipped = true;
        }
    }
    while let Ok(ev) = rx_j.try_recv() {
        if ev.message.contains("Ignoré") {
            skipped = true;
        }
    }
    assert!(skipped, "expected unchanged file to be skipped on re-transfer");
}
