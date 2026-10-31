use anyhow::{bail, Context, Result};
use peeroxide::{discovery_key, spawn, JoinOpts, KeyPair, SwarmConfig};
use peeroxide_dht::secret_stream::SecretStream;
use std::time::Duration;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

/// Both sides derive the same rendezvous topic from the server's public key.
fn topic_for(server_public_key: &[u8; 32]) -> [u8; 32] {
    let mut input = b"hyperssh/v1/".to_vec();
    input.extend_from_slice(server_public_key);
    discovery_key(&input)
}

/// Copy bytes between an encrypted peer stream and a local reader/writer pair.
///
/// Returns when the remote side closes. A local EOF only half-closes the
/// peer stream, so the remote can still finish sending (e.g. ssh's last bytes).
async fn bridge<T, R, W>(stream: SecretStream<T>, mut local_r: R, mut local_w: W) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (mut peer_r, mut peer_w) = stream.into_split();

    let up = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = local_r.read(&mut buf).await?;
            if n == 0 {
                peer_w.shutdown().await?;
                break;
            }
            // SecretStream::write sends the whole slice as one framed message.
            peer_w.write(&buf[..n]).await?;
        }
        Ok::<(), anyhow::Error>(())
    };

    let down = async {
        while let Some(data) = peer_r.read().await? {
            local_w.write_all(&data).await?;
            local_w.flush().await?;
        }
        let _ = local_w.shutdown().await;
        Ok::<(), anyhow::Error>(())
    };

    tokio::pin!(up);
    tokio::pin!(down);
    let mut up_done = false;
    loop {
        tokio::select! {
            r = &mut up, if !up_done => { r?; up_done = true; }
            r = &mut down => { r?; break; }
        }
    }
    Ok(())
}

/// Client side: find the server by public key and pipe stdin/stdout through it.
/// Intended to run as ssh's `ProxyCommand`, so stdout carries ONLY tunnel bytes.
pub async fn connect_stdio(client_identity: KeyPair, server_public_key: [u8; 32]) -> Result<()> {
    let mut config = SwarmConfig::with_public_bootstrap();
    config.key_pair = Some(client_identity);

    let (_task, handle, mut conn_rx) = spawn(config).await.context("Failed to start swarm")?;

    let mut opts = JoinOpts::default();
    opts.server = false;
    opts.client = true;
    handle
        .join(topic_for(&server_public_key), opts)
        .await
        .context("Failed to join topic")?;

    eprintln!("Looking for server {}...", hex::encode(server_public_key));

    let conn = tokio::time::timeout(CONNECT_TIMEOUT, async {
        while let Some(conn) = conn_rx.recv().await {
            // Anyone can announce on a topic; only accept the key we asked for.
            if AsRef::<[u8]>::as_ref(&conn.remote_public_key()) == &server_public_key[..] {
                return Some(conn);
            }
        }
        None
    })
    .await
    .context("Timed out looking for server")?
    .context("Swarm closed before connecting")?;

    eprintln!("Connected.");
    bridge(conn.peer.stream, io::stdin(), io::stdout()).await
}

/// Server side: accept tunnels and bridge each one to the local sshd.
pub async fn serve(identity: KeyPair, ssh_addr: String) -> Result<()> {
    let public_key = identity.public_key;

    let mut config = SwarmConfig::with_public_bootstrap();
    config.key_pair = Some(identity);

    let (task, handle, mut conn_rx) = spawn(config).await.context("Failed to start swarm")?;

    let mut opts = JoinOpts::default();
    opts.server = true;
    opts.client = false;
    handle
        .join(topic_for(&public_key), opts)
        .await
        .context("Failed to announce server")?;

    eprintln!("hyperssh is serving. id: {}", hex::encode(public_key));
    eprintln!("forwarding to {ssh_addr}");

    while let Some(conn) = conn_rx.recv().await {
        let ssh_addr = ssh_addr.clone();
        tokio::spawn(async move {
            let peer = hex::encode(AsRef::<[u8]>::as_ref(&conn.remote_public_key()));
            eprintln!("[{peer:.8}] connected");
            let result = async {
                let tcp = TcpStream::connect(&ssh_addr)
                    .await
                    .with_context(|| format!("Could not reach sshd at {ssh_addr}"))?;
                let (r, w) = tcp.into_split();
                bridge(conn.peer.stream, r, w).await
            }
            .await;
            match result {
                Ok(()) => eprintln!("[{peer:.8}] closed"),
                Err(e) => eprintln!("[{peer:.8}] error: {e:#}"),
            }
        });
    }

    let _ = task.await;
    bail!("Swarm stopped unexpectedly")
}