use std::{
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use p5099_core::client_manifest::{decode_key, is_safe_relative_path};
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
    time::timeout,
};

const MAX_CONNECTIONS: usize = 64;
const MAX_REQUEST_BYTES: usize = 8 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_STALL_TIMEOUT: Duration = Duration::from_mins(1);
const CHUNK_BYTES: usize = 256 * 1024;
const LINGER: Duration = Duration::from_secs(2);

pub struct DownloadServer {
    listener: TcpListener,
    root: PathBuf,
}

impl DownloadServer {
    pub async fn bind(address: SocketAddr, root: &Path) -> io::Result<Self> {
        let root = tokio::fs::canonicalize(root).await?;
        if !tokio::fs::metadata(&root).await?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a directory", root.display()),
            ));
        }
        let listener = TcpListener::bind(address).await?;
        Ok(Self { listener, root })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn run(self) {
        let root = Arc::new(self.root);
        let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        loop {
            let (stream, peer) = match self.listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!(%error, "download listener failed to accept");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
            };
            let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                tracing::debug!(%peer, "download connection refused: too many connections");
                continue;
            };
            let root = Arc::clone(&root);
            tokio::spawn(async move {
                let _permit = permit;
                if let Err(error) = serve(stream, &root).await {
                    tracing::debug!(%peer, %error, "download connection ended early");
                }
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Get,
    Head,
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, root: &Path) -> io::Result<()> {
    respond(&mut stream, root).await?;
    stream.shutdown().await?;
    let _ = timeout(LINGER, async {
        let mut buffer = [0_u8; 4096];
        while matches!(stream.read(&mut buffer).await, Ok(read) if read > 0) {}
    })
    .await;
    Ok(())
}

async fn respond<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, root: &Path) -> io::Result<()> {
    let head = match timeout(REQUEST_TIMEOUT, read_head(stream)).await {
        Ok(Ok(Some(head))) => head,
        Ok(Ok(None)) => return respond_empty(stream, "400 Bad Request").await,
        Ok(Err(error)) => return Err(error),
        Err(_) => return Ok(()),
    };
    let mut parts = head.lines().next().unwrap_or_default().split(' ');
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return respond_empty(stream, "400 Bad Request").await;
    };
    if !version.starts_with("HTTP/1.") {
        return respond_empty(stream, "400 Bad Request").await;
    }
    let method = match method {
        "GET" => Method::Get,
        "HEAD" => Method::Head,
        _ => {
            return write_all(
                stream,
                b"HTTP/1.1 405 Method Not Allowed\r\nAllow: GET, HEAD\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        }
    };
    let Some(path) = resolve(root, target).await else {
        return respond_empty(stream, "404 Not Found").await;
    };
    let mut file = File::open(&path).await?;
    let length = file.metadata().await?.len();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nContent-Type: application/octet-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    );
    write_all(stream, header.as_bytes()).await?;
    if method == Method::Get {
        let mut buffer = vec![0_u8; CHUNK_BYTES];
        loop {
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            write_all(stream, &buffer[..read]).await?;
        }
    }
    Ok(())
}

async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<Option<String>> {
    let mut head = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if head.len() > MAX_REQUEST_BYTES {
            return Ok(None);
        }
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        head.extend_from_slice(&buffer[..read]);
    }
    Ok(String::from_utf8(head).ok())
}

async fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    let encoded = target.split(['?', '#']).next()?.strip_prefix('/')?;
    let key = decode_key(encoded)?;
    if !is_safe_relative_path(&key) || key.split('/').any(|segment| segment.starts_with('.')) {
        return None;
    }
    let path = tokio::fs::canonicalize(root.join(&key)).await.ok()?;
    if !path.starts_with(root) || !tokio::fs::metadata(&path).await.ok()?.is_file() {
        return None;
    }
    Some(path)
}

async fn respond_empty<S: AsyncWrite + Unpin>(stream: &mut S, status: &str) -> io::Result<()> {
    let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    write_all(stream, response.as_bytes()).await
}

async fn write_all<S: AsyncWrite + Unpin>(stream: &mut S, bytes: &[u8]) -> io::Result<()> {
    timeout(WRITE_STALL_TIMEOUT, stream.write_all(bytes))
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use tokio::net::TcpStream;

    use super::*;

    async fn request(address: SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8_lossy(&response).into_owned()
    }

    async fn get(address: SocketAddr, target: &str) -> String {
        request(
            address,
            &format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n"),
        )
        .await
    }

    #[tokio::test]
    async fn serves_files_and_refuses_everything_else() {
        let folder = tempdir().unwrap();
        let root = folder.path().join("dist");
        fs::create_dir_all(root.join("windows/Data")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("windows/Data/a b.rho"), b"track").unwrap();
        fs::write(root.join(".git/config"), b"secret").unwrap();
        fs::write(folder.path().join("outside.txt"), b"secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(folder.path().join("outside.txt"), root.join("link.txt"))
            .unwrap();

        let server = DownloadServer::bind("127.0.0.1:0".parse().unwrap(), &root)
            .await
            .unwrap();
        let address = server.local_addr().unwrap();
        let task = tokio::spawn(server.run());

        let response = get(address, "/windows/Data/a%20b.rho").await;
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(response.contains("Content-Length: 5\r\n"));
        assert!(response.ends_with("\r\n\r\ntrack"));

        let response = request(
            address,
            "HEAD /windows/Data/a%20b.rho HTTP/1.1\r\nHost: x\r\n\r\n",
        )
        .await;
        assert!(response.contains("Content-Length: 5\r\n"));
        assert!(response.ends_with("\r\n\r\n"), "{response}");

        for target in [
            "/missing.txt",
            "/windows",
            "/../outside.txt",
            "/windows/%2E%2E/%2E%2E/outside.txt",
            "/.git/config",
            "/link.txt",
            "/windows\\Data",
            "relative",
            "/%ZZ",
        ] {
            let response = get(address, target).await;
            assert!(
                response.starts_with("HTTP/1.1 404 "),
                "{target}: {response}"
            );
        }

        let response = request(address, "POST /windows HTTP/1.1\r\n\r\n").await;
        assert!(response.starts_with("HTTP/1.1 405 "), "{response}");
        let response = request(address, "garbage\r\n\r\n").await;
        assert!(response.starts_with("HTTP/1.1 400 "), "{response}");
        let oversized = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(10_000));
        let response = request(address, &oversized).await;
        assert!(response.starts_with("HTTP/1.1 400 "), "{response}");

        task.abort();
    }
}
