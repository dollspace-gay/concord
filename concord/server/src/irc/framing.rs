use std::io;

use tokio::io::{AsyncRead, AsyncReadExt};

/// Maximum encoded size of an inbound IRC line, including its line terminator.
pub const MAX_LINE_LENGTH: usize = 4096;

/// Incrementally frames IRC lines without borrowing an async reader's internal buffer.
///
/// Both CRLF and bare LF are accepted for compatibility. The terminator counts toward
/// [`MAX_LINE_LENGTH`], and returned strings do not contain it.
#[derive(Debug, Default)]
pub struct IrcLineDecoder {
    buffered: Vec<u8>,
}

impl IrcLineDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the next complete line. Clean EOF returns `None`; EOF in a partial line,
    /// malformed UTF-8, and lines over the configured bound are protocol errors.
    pub async fn read_line<R>(&mut self, reader: &mut R) -> io::Result<Option<String>>
    where
        R: AsyncRead + Unpin,
    {
        loop {
            if let Some(newline) = self.buffered.iter().position(|byte| *byte == b'\n') {
                let encoded_len = newline + 1;
                if encoded_len > MAX_LINE_LENGTH {
                    self.buffered.clear();
                    return Err(line_too_long());
                }

                let mut encoded = self.buffered.drain(..encoded_len).collect::<Vec<_>>();
                encoded.pop();
                if encoded.last() == Some(&b'\r') {
                    encoded.pop();
                }
                return String::from_utf8(encoded).map(Some).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid IRC UTF-8: {error}"),
                    )
                });
            }

            // With no terminator yet, a buffer this large can no longer form a valid line.
            if self.buffered.len() >= MAX_LINE_LENGTH {
                self.buffered.clear();
                return Err(line_too_long());
            }

            let mut chunk = [0_u8; 1024];
            let read = reader.read(&mut chunk).await?;
            if read == 0 {
                if self.buffered.is_empty() {
                    return Ok(None);
                }
                self.buffered.clear();
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "IRC connection ended within a line",
                ));
            }
            self.buffered.extend_from_slice(&chunk[..read]);
        }
    }
}

fn line_too_long() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "IRC line exceeds maximum encoded length",
    )
}
