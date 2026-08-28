//! The host side of the length-prefixed frame codec `munibot_toolagent`
//! speaks over its unix socket.
//!
//! Hand-mirrored from `munibot_toolagent/src/codec.rs` for the same reason
//! `sandbox::protocol` mirrors that crate's wire types - see that module's
//! own doc comment.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The largest payload this codec will ever read or write. Must match
/// `munibot_toolagent::codec::MAX_FRAME_SIZE` exactly, or a frame one side
/// considers valid could be rejected by the other.
pub const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

/// What can go wrong decoding or encoding one frame.
#[derive(thiserror::Error, Debug)]
pub enum FrameError {
    /// The connection closed, or otherwise ran out of bytes, before a
    /// complete frame arrived.
    #[error("the connection closed before a complete frame arrived")]
    Truncated,
    /// A frame's declared length is larger than [`MAX_FRAME_SIZE`].
    #[error("a frame declared {declared} bytes, over the {MAX_FRAME_SIZE} byte limit")]
    TooLarge { declared: u32 },
    /// Every other I/O failure reading or writing the underlying stream.
    #[error("i/o error framing a message: {0}")]
    Io(#[from] std::io::Error),
}

/// Writes one frame: a four-byte big-endian length prefix, then `payload`
/// itself.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> Result<(), FrameError> {
    let len =
        u32::try_from(payload.len()).map_err(|_| FrameError::TooLarge { declared: u32::MAX })?;
    if len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge { declared: len });
    }

    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads one frame, returning its payload. Checks the declared length
/// against [`MAX_FRAME_SIZE`] before allocating a buffer for it.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, FrameError> {
    let mut len_bytes = [0u8; 4];
    reader
        .read_exact(&mut len_bytes)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::UnexpectedEof => FrameError::Truncated,
            _ => FrameError::Io(error),
        })?;

    let len = u32::from_be_bytes(len_bytes);
    if len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge { declared: len });
    }

    let mut payload = vec![0u8; len as usize];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::UnexpectedEof => FrameError::Truncated,
            _ => FrameError::Io(error),
        })?;

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_a_frame_round_trips_through_encode_and_decode() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, b"hello sandbox")
            .await
            .expect("should write");

        let decoded = read_frame(&mut &buffer[..]).await.expect("should read");
        assert_eq!(decoded, b"hello sandbox");
    }

    #[tokio::test]
    async fn test_reading_a_frame_with_a_missing_length_prefix_is_truncated() {
        let error = read_frame(&mut &b""[..]).await.expect_err("should fail");
        assert!(matches!(error, FrameError::Truncated), "got {error:?}");
    }

    #[tokio::test]
    async fn test_reading_a_frame_over_the_size_limit_is_rejected_without_allocating() {
        let mut buffer = (MAX_FRAME_SIZE + 1).to_be_bytes().to_vec();
        buffer.extend_from_slice(b"not even close to that many bytes");

        let error = read_frame(&mut &buffer[..]).await.expect_err("should fail");
        match error {
            FrameError::TooLarge { declared } => assert_eq!(declared, MAX_FRAME_SIZE + 1),
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_two_frames_written_back_to_back_are_read_independently() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, b"first").await.unwrap();
        write_frame(&mut buffer, b"second").await.unwrap();

        let mut cursor = &buffer[..];
        let first = read_frame(&mut cursor).await.expect("should read first");
        let second = read_frame(&mut cursor).await.expect("should read second");

        assert_eq!(first, b"first");
        assert_eq!(second, b"second");
    }
}
