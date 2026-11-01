//! The data-stream actor: turns a CSV file into an async stream of messages.

use std::path::Path;

use async_stream::try_stream;
use csv::{ReaderBuilder, StringRecord, Trim};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_stream::Stream;

use crate::message::Message;

pub mod run;

/// Failure while reading or parsing the input file.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// The file could not be opened or read.
    #[error("failed to read input file")]
    Io(#[from] std::io::Error),
    /// A row could not be parsed into a [`Message`].
    #[error("failed to parse csv record")]
    Csv(#[from] csv::Error),
}

/// Parses one line of CSV into a record, trimming whitespace around fields.
///
/// Rows are allowed to omit trailing fields (e.g. `dispute,1,1` without the
/// empty `amount`), which the deserializer maps to `None`.
fn parse_record(line: &str) -> csv::Result<StringRecord> {
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .trim(Trim::All)
        .flexible(true)
        .buffer_capacity(line.len() + 1)
        .from_reader(line.as_bytes());
    let mut record = StringRecord::new();
    reader.read_record(&mut record)?;
    Ok(record)
}

/// Opens `path` and yields one [`Message`] per CSV row, in file order.
///
/// The first non-blank line is the header; blank lines are skipped. The
/// stream ends after the first error (an unreadable file or a malformed row).
pub fn message_stream(
    path: impl AsRef<Path> + Send + 'static,
) -> impl Stream<Item = Result<Message, StreamError>> + Send {
    try_stream! {
        let file = File::open(path.as_ref()).await?;
        let mut lines = BufReader::new(file).lines();

        let mut headers: Option<StringRecord> = None;
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let record = parse_record(&line)?;
            match &headers {
                None => headers = Some(record),
                Some(headers) => {
                    let message: Message = record.deserialize(Some(headers))?;
                    yield message;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_stream::run;
    use crate::message::MessageType;
    use rust_decimal::Decimal;
    use std::path::PathBuf;
    use tokio_stream::StreamExt;

    struct TempCsv {
        path: PathBuf,
    }

    impl TempCsv {
        fn new(name: &str, contents: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "toymachine-{}-{}-{name}.csv",
                std::process::id(),
                std::thread::current()
                    .name()
                    .unwrap_or("test")
                    .replace("::", "-")
            ));
            std::fs::write(&path, contents).expect("temp csv is writable");
            Self { path }
        }
    }

    impl Drop for TempCsv {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    async fn collect(path: &Path) -> Vec<Result<Message, StreamError>> {
        message_stream(path.to_path_buf()).collect().await
    }

    async fn collect_ok(path: &Path) -> Vec<Message> {
        collect(path)
            .await
            .into_iter()
            .map(|item| item.expect("every row should parse"))
            .collect()
    }

    #[tokio::test]
    async fn streams_every_row_in_file_order() {
        let file = TempCsv::new(
            "order",
            "type,client,tx,amount\ndeposit,1,1,1.0\nwithdrawal,1,2,0.5\n",
        );

        let messages = collect_ok(&file.path).await;

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].kind, MessageType::Deposit);
        assert_eq!(messages[0].amount, Some(Decimal::new(10, 1)));
        assert_eq!(messages[1].kind, MessageType::Withdrawal);
        assert_eq!(messages[1].tx, 2);
    }

    #[tokio::test]
    async fn whitespace_in_header_and_rows_is_tolerated() {
        let file = TempCsv::new(
            "whitespace",
            "type, client, tx, amount\n  deposit ,  1 ,  1 ,  1.2345 \n dispute, 1, 1,\n",
        );

        let messages = collect_ok(&file.path).await;

        assert_eq!(messages[0].amount, Some(Decimal::new(12345, 4)));
        assert_eq!(messages[1].kind, MessageType::Dispute);
        assert_eq!(messages[1].amount, None);
    }

    #[tokio::test]
    async fn dispute_row_without_trailing_comma_is_accepted() {
        let file = TempCsv::new("short-row", "type,client,tx,amount\ndispute,1,1\n");

        let messages = collect_ok(&file.path).await;

        assert_eq!(messages[0].kind, MessageType::Dispute);
        assert_eq!(messages[0].amount, None);
    }

    #[tokio::test]
    async fn blank_lines_are_skipped() {
        let file = TempCsv::new(
            "blank",
            "\ntype,client,tx,amount\n\ndeposit,1,1,1.0\n   \nwithdrawal,1,2,0.5\n\n",
        );

        let messages = collect_ok(&file.path).await;

        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn empty_file_yields_no_messages() {
        let file = TempCsv::new("empty", "");

        let items = collect(&file.path).await;

        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn header_only_file_yields_no_messages() {
        let file = TempCsv::new("header-only", "type,client,tx,amount\n");

        let items = collect(&file.path).await;

        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn missing_file_yields_single_io_error() {
        let path = std::env::temp_dir().join("toymachine-definitely-missing.csv");

        let items = collect(&path).await;

        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Err(StreamError::Io(_))));
    }

    #[tokio::test]
    async fn malformed_row_yields_csv_error_and_stops_the_stream() {
        let file = TempCsv::new(
            "malformed",
            "type,client,tx,amount\ndeposit,1,1,1.0\nrefund,1,2,1.0\ndeposit,1,3,1.0\n",
        );

        let items = collect(&file.path).await;

        assert_eq!(
            items.len(),
            2,
            "one good row, then the error ends the stream"
        );
        assert!(items[0].is_ok());
        assert!(matches!(items[1], Err(StreamError::Csv(_))));
    }

    #[tokio::test]
    async fn run_forwards_all_messages_into_channel_and_completes() {
        let file = TempCsv::new(
            "run",
            "type,client,tx,amount\ndeposit,1,1,1.0\ndeposit,2,2,2.0\ndeposit,3,3,3.0\n",
        );
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);

        let producer = tokio::spawn(run::run(file.path.clone(), sender));
        let mut received = Vec::new();
        while let Some(message) = receiver.recv().await {
            received.push(message);
        }

        producer
            .await
            .expect("producer task completes")
            .expect("producer succeeds");
        assert_eq!(received.len(), 3);
        assert_eq!(received[2].client, 3);
    }

    #[tokio::test]
    async fn run_returns_error_for_missing_file() {
        let path = std::env::temp_dir().join("toymachine-also-missing.csv");
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);

        let result = run::run(path, sender).await;

        assert!(matches!(result, Err(StreamError::Io(_))));
    }

    #[tokio::test]
    async fn run_stops_quietly_when_receiver_is_dropped() {
        let file = TempCsv::new(
            "dropped",
            "type,client,tx,amount\ndeposit,1,1,1.0\ndeposit,2,2,2.0\n",
        );
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(receiver);

        let result = run::run(file.path.clone(), sender).await;

        assert!(result.is_ok());
    }
}
