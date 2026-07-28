//! Bounded framing for the shared executor wire contract.
//!
//! Printer drivers and PDF renderers run out of process. This crate owns the
//! byte-stream boundary while [`spool_protocol::executor`] remains the single
//! semantic contract shared with the rest of the workspace.

use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use thiserror::Error;

pub use spool_protocol::executor::{
    DiscoveredPrinter, ExecutorError, ExecutorOperation, ExecutorRequest, ExecutorResponse,
    ExecutorResult,
};

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame length {0} exceeds the {MAX_FRAME_BYTES} byte limit")]
    TooLarge(usize),
    #[error("truncated frame")]
    Truncated,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON frame: {0}")]
    Json(#[from] serde_json::Error),
}

/// Serializes a semantic executor message into one bounded frame.
///
/// # Errors
///
/// Returns an error if JSON serialization fails or the body exceeds the
/// protocol limit.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(value)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(body.len()));
    }
    let mut output = Vec::with_capacity(4 + body.len());
    #[expect(
        clippy::cast_possible_truncation,
        reason = "bounded by MAX_FRAME_BYTES"
    )]
    output.put_u32(body.len() as u32);
    output.extend_from_slice(&body);
    Ok(output)
}

/// Decodes exactly one complete executor frame.
///
/// # Errors
///
/// Returns an error for a truncated, oversized, or invalid JSON frame.
pub fn decode_frame<T: for<'de> Deserialize<'de>>(frame: &[u8]) -> Result<T, FrameError> {
    if frame.len() < 4 {
        return Err(FrameError::Truncated);
    }
    let mut input = frame;
    let body_len =
        usize::try_from(input.get_u32()).map_err(|_| FrameError::TooLarge(usize::MAX))?;
    if body_len > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(body_len));
    }
    if input.len() != body_len {
        return Err(FrameError::Truncated);
    }
    Ok(serde_json::from_slice(input)?)
}

/// Writes and flushes exactly one framed executor message.
///
/// # Errors
///
/// Returns an error if serialization or stream I/O fails.
pub fn write_frame<T: Serialize>(mut writer: impl Write, value: &T) -> Result<(), FrameError> {
    writer.write_all(&encode_frame(value)?)?;
    writer.flush()?;
    Ok(())
}

/// Reads exactly one framed executor message.
///
/// # Errors
///
/// Returns an error for stream I/O, an oversized frame, or invalid JSON.
pub fn read_frame<T: for<'de> Deserialize<'de>>(mut reader: impl Read) -> Result<T, FrameError> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let body_len = usize::try_from(u32::from_be_bytes(header))
        .map_err(|_| FrameError::TooLarge(usize::MAX))?;
    if body_len > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(body_len));
    }
    let mut body = vec![0_u8; body_len];
    reader.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

/// Incremental decoder used when a child process stream splits or coalesces
/// frame writes.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buffer: BytesMut,
}

impl FrameDecoder {
    /// Pushes stream bytes and returns all complete frames.
    ///
    /// # Errors
    ///
    /// Returns an error before allocating a declared oversized frame, or if a
    /// complete frame is not valid JSON for `T`.
    pub fn push<T: for<'de> Deserialize<'de>>(
        &mut self,
        bytes: &[u8],
    ) -> Result<Vec<T>, FrameError> {
        self.buffer.extend_from_slice(bytes);
        let mut decoded = Vec::new();
        loop {
            if self.buffer.len() < 4 {
                return Ok(decoded);
            }
            let header: [u8; 4] = self.buffer[..4]
                .try_into()
                .map_err(|_| FrameError::Truncated)?;
            let body_len = usize::try_from(u32::from_be_bytes(header))
                .map_err(|_| FrameError::TooLarge(usize::MAX))?;
            if body_len > MAX_FRAME_BYTES {
                return Err(FrameError::TooLarge(body_len));
            }
            if self.buffer.len() < body_len + 4 {
                return Ok(decoded);
            }
            self.buffer.advance(4);
            let body = self.buffer.split_to(body_len);
            decoded.push(serde_json::from_slice(&body)?);
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use uuid::Uuid;

    fn request() -> ExecutorRequest {
        ExecutorRequest {
            request_id: Uuid::nil(),
            deadline_unix_ms: 42,
            operation: ExecutorOperation::DiscoverPrinters,
        }
    }

    #[test]
    fn round_trips_frame() {
        let original = request();
        let frame = encode_frame(&original).expect("encode");
        let decoded: ExecutorRequest = decode_frame(&frame).expect("decode");
        assert_eq!(decoded.request_id, original.request_id);
        assert!(matches!(
            decoded.operation,
            ExecutorOperation::DiscoverPrinters
        ));
    }

    #[test]
    fn incremental_decoder_handles_split_frames() {
        let frame = encode_frame(&request()).expect("encode");
        let mut decoder = FrameDecoder::default();
        assert!(
            decoder
                .push::<ExecutorRequest>(&frame[..3])
                .expect("decode")
                .is_empty()
        );
        assert!(
            decoder
                .push::<ExecutorRequest>(&frame[3..8])
                .expect("decode")
                .is_empty()
        );
        let messages = decoder
            .push::<ExecutorRequest>(&frame[8..])
            .expect("decode");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].request_id, Uuid::nil());
    }

    #[test]
    fn rejects_advertised_oversized_frame_before_allocating() {
        #[expect(clippy::cast_possible_truncation, reason = "test limit fits u32")]
        let frame = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes();
        assert!(matches!(
            decode_frame::<ExecutorRequest>(&frame),
            Err(FrameError::TooLarge(_))
        ));
    }

    proptest! {
        #[test]
        fn arbitrary_chunks_never_panic(chunks in prop::collection::vec(any::<u8>(), 0..4096)) {
            let mut decoder = FrameDecoder::default();
            let _ = decoder.push::<ExecutorRequest>(&chunks);
        }
    }
}
