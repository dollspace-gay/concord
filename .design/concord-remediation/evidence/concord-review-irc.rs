use tokio::io::{AsyncRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
const MAX_LINE_LENGTH: usize = 4096;
async fn read_bounded_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    buf: &mut String,
) -> std::io::Result<usize> {
    // Fill the internal buffer and check for a newline within MAX_LINE_LENGTH
    loop {
        let available = reader.buffer();
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            // Found newline within buffered data
            let line_bytes = &available[..=pos];
            let line = String::from_utf8_lossy(line_bytes).into_owned();
            let len = line_bytes.len();
            buf.push_str(&line);
            reader.consume(len);
            return Ok(len);
        }
        if available.len() >= MAX_LINE_LENGTH {
            // Too long without a newline — discard and signal error
            let discard_len = available.len();
            reader.consume(discard_len);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "IRC line exceeds maximum length",
            ));
        }
        // Need more data
        let filled = reader.fill_buf().await?;
        if filled.is_empty() {
            return Ok(0); // EOF
        }
    }
}


#[tokio::main(flavor="current_thread")]
async fn main() {
    let (mut writer, reader) = tokio::io::duplex(8192);
    let mode = std::env::args().nth(1).unwrap();
    if mode == "fragment" {
        writer.write_all(b"NI").await.unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            writer.write_all(b"CK owner\r\n").await.unwrap();
        });
    } else {
        writer.write_all(b"NICK owner\r\n").await.unwrap();
    }
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let n = read_bounded_line(&mut reader, &mut line).await.unwrap();
    println!("read {n} bytes");
}
