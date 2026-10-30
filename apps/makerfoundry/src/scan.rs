//! Virus scanning (issue #29): every 3D upload's files go to a ClamAV daemon before they're kept,
//! when `MF_CLAMAV` names one (`host:port`); without it nothing changes. Spoken from clamd's
//! documented protocol (<https://docs.clamav.net/manual/Usage/Scanning.html#clamd>, `INSTREAM`):
//! `zINSTREAM\0`, then the file in chunks each led by its length as a big-endian u32, then a zero
//! length; clamd answers `stream: OK`, `stream: <name> FOUND` or an error.
//!
//! A file clamd flags is refused. If clamd can't be reached or answers anything else, uploads are
//! held (refused, asking to try later) rather than kept unscanned.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// How long a scan may take, all told.
const TIMEOUT: Duration = Duration::from_secs(60);
/// The size of each chunk sent.
const CHUNK: usize = 64 * 1024;
/// The longest answer read.
const MAX_ANSWER: u64 = 1024;

/// Where clamd listens.
#[derive(Clone, Debug)]
pub struct Scanner {
    address: String,
}

/// What a scan found.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Clean,
    /// Flagged, with the name clamd gave it.
    Found(String),
}

impl Scanner {
    /// From `MF_CLAMAV`: `host:port`.
    pub fn new(address: &str) -> Option<Self> {
        let address = address.trim();
        (!address.is_empty() && address.contains(':')).then(|| Scanner {
            address: address.to_string(),
        })
    }

    /// Scans `bytes` → its verdict, or why it couldn't be scanned.
    pub async fn scan(&self, bytes: &[u8]) -> Result<Verdict, String> {
        tokio::time::timeout(TIMEOUT, self.scan_now(bytes))
            .await
            .map_err(|_| "the virus scanner took too long".to_string())?
    }

    async fn scan_now(&self, bytes: &[u8]) -> Result<Verdict, String> {
        let down = |e: std::io::Error| format!("the virus scanner can't be reached: {e}");
        let mut stream = TcpStream::connect(&self.address).await.map_err(down)?;
        stream.write_all(b"zINSTREAM\0").await.map_err(down)?;
        for chunk in bytes.chunks(CHUNK) {
            let len = u32::try_from(chunk.len()).map_err(|_| "a chunk too large".to_string())?;
            stream.write_all(&len.to_be_bytes()).await.map_err(down)?;
            stream.write_all(chunk).await.map_err(down)?;
        }
        stream.write_all(&0u32.to_be_bytes()).await.map_err(down)?;
        let mut answer = Vec::new();
        (&mut stream)
            .take(MAX_ANSWER)
            .read_to_end(&mut answer)
            .await
            .map_err(down)?;
        let answer = String::from_utf8_lossy(&answer);
        let answer = answer.trim_end_matches(['\0', '\n']).trim();
        let body = answer.strip_prefix("stream:").map(str::trim).unwrap_or(answer);
        if body == "OK" {
            Ok(Verdict::Clean)
        } else if let Some(name) = body.strip_suffix("FOUND") {
            Ok(Verdict::Found(name.trim().chars().take(100).collect()))
        } else {
            Err(format!(
                "the virus scanner answered: {}",
                body.chars().take(100).collect::<String>()
            ))
        }
    }
}

/// A stand-in clamd for tests (here and in `tests/`): flags anything containing `marker`, and
/// answers nothing at all once `down` is set.
#[doc(hidden)]
pub mod stand_in {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Starts one → its address and its "down" switch.
    pub async fn start(marker: &'static [u8]) -> std::io::Result<(String, Arc<AtomicBool>)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?.to_string();
        let down = Arc::new(AtomicBool::new(false));
        let flag = down.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                if flag.load(Ordering::SeqCst) {
                    continue; // (dropped: the scanner is "down")
                }
                tokio::spawn(async move {
                    let mut command = [0u8; 10];
                    if socket.read_exact(&mut command).await.is_err() || &command != b"zINSTREAM\0" {
                        let _ = socket.write_all(b"UNKNOWN COMMAND\0").await;
                        return;
                    }
                    let mut data = Vec::new();
                    loop {
                        let mut len = [0u8; 4];
                        if socket.read_exact(&mut len).await.is_err() {
                            return;
                        }
                        let n = u32::from_be_bytes(len) as usize;
                        if n == 0 {
                            break;
                        }
                        let mut chunk = vec![0u8; n.min(1 << 20)];
                        if socket.read_exact(&mut chunk).await.is_err() {
                            return;
                        }
                        data.extend(chunk);
                    }
                    let found = data.windows(marker.len()).any(|w| w == marker);
                    let answer: &[u8] = if found {
                        b"stream: Test-Signature FOUND\0"
                    } else {
                        b"stream: OK\0"
                    };
                    let _ = socket.write_all(answer).await;
                });
            }
        });
        Ok((address, down))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the stand-in flags (not the real EICAR string, which isn't kept in the repository).
    const MARKER: &[u8] = b"MF-TEST-VIRUS-SIGNATURE";

    #[tokio::test]
    async fn a_flagged_file_is_found_and_a_clean_one_passes() {
        let (address, down) = stand_in::start(MARKER).await.unwrap();
        let scanner = Scanner::new(&address).unwrap();
        let mut big = vec![b'x'; 300_000];
        big.extend(MARKER);
        assert_eq!(
            scanner.scan(&big).await.unwrap(),
            Verdict::Found("Test-Signature".into())
        );
        assert_eq!(
            scanner.scan(b"ISO-10303-21; a clean STEP").await.unwrap(),
            Verdict::Clean
        );
        assert_eq!(scanner.scan(b"").await.unwrap(), Verdict::Clean);
        down.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(scanner.scan(b"anything").await.is_err(), "down: no verdict");
    }

    #[tokio::test]
    async fn an_unreachable_scanner_is_an_error() {
        let scanner = Scanner::new("127.0.0.1:9").unwrap();
        assert!(scanner.scan(b"x").await.is_err());
        assert!(Scanner::new("").is_none() && Scanner::new("nohost").is_none());
    }
}
