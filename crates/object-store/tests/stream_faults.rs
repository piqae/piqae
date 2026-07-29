#![allow(
    clippy::expect_used,
    reason = "fault-test setup and assertions must fail immediately with local context"
)]

use bytes::Bytes;
use futures::stream;
use sha2::{Digest, Sha256};
use spool_object_store::{
    FileObjectStore, ObjectByteStream, ObjectStore, ObjectStoreError, digest_hex,
};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn temporary_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "spool-object-fault-{label}-{}-{unique}",
        std::process::id()
    ))
}

async fn remove_test_root(root: &PathBuf) {
    if let Err(error) = tokio::fs::remove_dir_all(root).await {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::NotFound,
            "remove isolated test directory"
        );
    }
}

#[tokio::test]
async fn interrupted_replacement_preserves_the_verified_object_and_removes_partial_data() {
    let root = temporary_root("interrupted");
    let store = FileObjectStore::new(&root).await.expect("file store");
    let key = "workspace/document.pdf";
    let original = Bytes::from_static(b"previous verified document");
    store
        .put(key, original.clone(), Some(&digest_hex(&original)))
        .await
        .expect("seed object");

    let replacement = Bytes::from_static(b"replacement document");
    let interrupted: ObjectByteStream = Box::pin(stream::iter([
        Ok(Bytes::from_static(b"replacement ")),
        Err(ObjectStoreError::Stream("injected disconnect".into())),
    ]));
    let result = store
        .put_stream(
            key,
            interrupted,
            &digest_hex(&replacement),
            replacement.len() as u64,
        )
        .await;
    assert!(matches!(result, Err(ObjectStoreError::Stream(_))));
    assert_eq!(store.get(key).await.expect("original survives"), original);

    let mut entries = tokio::fs::read_dir(root.join("workspace"))
        .await
        .expect("workspace directory");
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.expect("directory entry") {
        names.push(entry.file_name());
    }
    assert_eq!(names, [std::ffi::OsString::from("document.pdf")]);
    remove_test_root(&root).await;
}

#[tokio::test]
async fn digest_or_length_failure_never_publishes_a_new_object() {
    let root = temporary_root("verification");
    let store = FileObjectStore::new(&root).await.expect("file store");
    let content = Bytes::from_static(b"label payload");

    let wrong_digest: ObjectByteStream = Box::pin(stream::once(async {
        Ok(Bytes::from_static(b"label payload"))
    }));
    assert!(matches!(
        store
            .put_stream("workspace/digest", wrong_digest, &"0".repeat(64), 13)
            .await,
        Err(ObjectStoreError::DigestMismatch)
    ));
    assert!(
        !store
            .exists("workspace/digest")
            .await
            .expect("digest absent")
    );

    let truncated: ObjectByteStream =
        Box::pin(stream::once(async { Ok(Bytes::from_static(b"label")) }));
    assert!(matches!(
        store
            .put_stream(
                "workspace/truncated",
                truncated,
                &digest_hex(&content),
                content.len() as u64
            )
            .await,
        Err(ObjectStoreError::LengthMismatch)
    ));
    assert!(
        !store
            .exists("workspace/truncated")
            .await
            .expect("truncated absent")
    );
    remove_test_root(&root).await;
}

#[tokio::test]
async fn multi_chunk_document_is_verified_without_a_single_request_body_buffer() {
    const CHUNK_BYTES: usize = 64 * 1024;
    const CHUNKS: usize = 128;

    let root = temporary_root("multi-chunk");
    let store = FileObjectStore::new(&root).await.expect("file store");
    let chunks: Vec<_> = (0..CHUNKS)
        .map(|index| Bytes::from(vec![u8::try_from(index).expect("index fits"); CHUNK_BYTES]))
        .collect();
    let mut hasher = Sha256::new();
    for chunk in &chunks {
        hasher.update(chunk);
    }
    let digest = format!("{:x}", hasher.finalize());
    let expected_bytes = (CHUNK_BYTES * CHUNKS) as u64;
    let stream: ObjectByteStream = Box::pin(stream::iter(chunks.into_iter().map(Ok)));

    let stored = store
        .put_stream("workspace/large.pdf", stream, &digest, expected_bytes)
        .await
        .expect("streamed object");
    assert_eq!(stored.bytes, expected_bytes);
    assert_eq!(
        store
            .verify("workspace/large.pdf", &digest, expected_bytes)
            .await
            .expect("streamed verification"),
        stored
    );
    remove_test_root(&root).await;
}
