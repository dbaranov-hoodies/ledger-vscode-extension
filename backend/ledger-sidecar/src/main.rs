//! Reads requests from the standard input and writes answers and notifications to the standard
//! output, one JSON value per line. Anything meant for a human goes to the standard error.

use ledger_runtime::SystemRunner;
use ledger_sidecar::Server;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let (server, mut lines) = Server::new(SystemRunner);

    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(line) = lines.recv().await {
            stdout.write_all(line.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
        Ok::<_, std::io::Error>(())
    });

    let mut requests = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = requests.next_line().await? {
        if !line.trim().is_empty() {
            server.handle(&line).await;
        }
    }

    // The extension closed the input: let the writer say what is left, then stop.
    drop(server);
    let _ = tokio::time::timeout(std::time::Duration::from_millis(200), writer).await;
    Ok(())
}
