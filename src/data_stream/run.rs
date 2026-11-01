use crate::{Message, StreamError, data_stream};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

/// Actor loop: streams `path` and forwards every message into `sender`.
///
/// Returns once the file is exhausted or the receiving side has gone away.
///
/// # Errors
///
/// Returns the first [`StreamError`] produced by [`data_stream::message_stream`].
pub async fn run(path: PathBuf, sender: mpsc::Sender<Message>) -> Result<(), StreamError> {
    let stream = data_stream::message_stream(path);
    tokio::pin!(stream);
    while let Some(message) = stream.next().await {
        if sender.send(message?).await.is_err() {
            // The accountant stopped listening; nothing left to do.
            break;
        }
    }
    Ok(())
}
