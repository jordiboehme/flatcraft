//! The stdio transport: one client on stdin and stdout.

use rmcp::ServiceExt;
use rmcp::service::ServerInitializeError;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{FlatcraftMcp, OnChange, SharedSession};

/// Serves the tools on any reader and writer pair until the reader closes.
/// A client that closes before the handshake is a normal end.
pub async fn serve_with<R, W>(
    session: SharedSession,
    on_change: OnChange,
    reader: R,
    writer: W,
) -> anyhow::Result<()>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let server = FlatcraftMcp::new(session, on_change);
    let running = match server.serve((reader, writer)).await {
        Ok(running) => running,
        Err(ServerInitializeError::ConnectionClosed(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    running.waiting().await?;
    Ok(())
}

/// Serves on the process's stdin and stdout on a tokio runtime and returns
/// when the client closes stdin.
pub fn serve(session: SharedSession, on_change: OnChange) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (stdin, stdout) = rmcp::transport::io::stdio();
    let result = runtime.block_on(serve_with(session, on_change, stdin, stdout));
    // A pending stdin read cannot be cancelled and would hold the runtime's
    // shutdown; the client is gone, so do not wait for it.
    runtime.shutdown_background();
    result
}
