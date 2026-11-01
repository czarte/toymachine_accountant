use anyhow::Context;
use std::io;
use std::path::PathBuf;
use tokio::sync::mpsc;
use toymachine::accountant::{reporter, run};
use toymachine::data_stream;

/// Maximum number of messages buffered between the two actors.
const CHANNEL_CAPACITY: usize = 1024;

pub async fn process(path: PathBuf) -> anyhow::Result<()> {
    let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
    let producer = tokio::spawn(data_stream::run::run(path, sender));
    let consumer = tokio::spawn(run::run(receiver));

    let (stream_result, clients) =
        tokio::try_join!(producer, consumer).context("an actor task panicked or was cancelled")?;
    stream_result?;

    reporter::write_report(&clients, io::stdout().lock()).context("failed to write report")?;
    Ok(())
}
