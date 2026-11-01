//! Integration tests: the two actors wired together through the channel, and
//! the compiled binary driven end to end.

use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;

use rust_decimal::Decimal;
use tokio::sync::mpsc;

use toymachine::accountant::clients::ClientMap;
use toymachine::accountant::{reporter, run};
use toymachine::data_stream;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).expect("valid decimal literal")
}

/// Runs `data_stream` and `accountant` concurrently, exactly like `main` does.
async fn run_pipeline(path: PathBuf) -> Result<ClientMap, data_stream::StreamError> {
    let (sender, receiver) = mpsc::channel(8);
    let producer = tokio::spawn(data_stream::run::run(path, sender));
    let consumer = tokio::spawn(run::run(receiver));

    let (stream_result, clients) = tokio::join!(producer, consumer);
    stream_result.expect("producer task completes")?;
    Ok(clients.expect("consumer task completes"))
}

fn render(clients: &ClientMap) -> String {
    let mut buffer = Vec::new();
    reporter::write_report(clients, &mut buffer).expect("writing to a Vec cannot fail");
    String::from_utf8(buffer).expect("csv output is utf-8")
}

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_toymachine"))
}

// ------------------------------------------------------------ actors wired

#[tokio::test]
async fn pipeline_computes_balances_for_basic_fixture() {
    let clients = run_pipeline(fixture("basic.csv"))
        .await
        .expect("pipeline succeeds");

    assert_eq!(clients.len(), 2);
    assert_eq!(clients[&1].available, dec("1.5"));
    assert_eq!(clients[&1].total, dec("1.5"));
    assert_eq!(clients[&2].available, dec("2.0"));
    assert_eq!(clients[&2].total, dec("2.0"));
    assert!(!clients[&1].locked && !clients[&2].locked);
}

#[tokio::test]
async fn pipeline_applies_disputes_resolves_and_chargebacks() {
    let clients = run_pipeline(fixture("disputes.csv"))
        .await
        .expect("pipeline succeeds");

    assert_eq!(
        render(&clients),
        "client,available,held,total,locked\n\
         1,10,0,10,true\n\
         2,1.2,0,1.2,false\n"
    );
}

#[tokio::test]
async fn pipeline_replays_stashed_dispute_when_deposit_arrives() {
    let clients = run_pipeline(fixture("stash.csv"))
        .await
        .expect("pipeline succeeds");

    assert_eq!(
        render(&clients),
        "client,available,held,total,locked\n\
         1,0,5,5,false\n\
         2,1,0,1,false\n"
    );
}

#[tokio::test]
async fn pipeline_reports_malformed_rows_as_error() {
    let result = run_pipeline(fixture("malformed.csv")).await;

    assert!(matches!(result, Err(data_stream::StreamError::Csv(_))));
}

#[tokio::test]
async fn pipeline_reports_missing_file_as_io_error() {
    let result = run_pipeline(fixture("does-not-exist.csv")).await;

    assert!(matches!(result, Err(data_stream::StreamError::Io(_))));
}

// ------------------------------------------------------------------ binary

#[test]
fn binary_prints_report_for_project_input_csv() {
    let output = binary()
        .arg(fixture("input.csv"))
        .output()
        .expect("binary runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "client,available,held,total,locked\n1,1.5,0,1.5,false\n2,2,0,2,false\n"
    );
}

#[test]
fn binary_prints_report_for_disputes_fixture() {
    let output = binary()
        .arg(fixture("disputes.csv"))
        .output()
        .expect("binary runs");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "client,available,held,total,locked\n1,10,0,10,true\n2,1.2,0,1.2,false\n"
    );
}

#[test]
fn binary_without_arguments_prints_usage_and_fails() {
    let output = binary().output().expect("binary runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "nothing is written to stdout");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Usage: cargo run -- <transactions.csv> > <output.csv>")
    );
}

#[test]
fn binary_with_too_many_arguments_prints_usage_and_fails() {
    let output = binary()
        .args(["a.csv", "b.csv"])
        .output()
        .expect("binary runs");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
}

#[test]
fn binary_with_missing_file_fails_with_error_message() {
    let output = binary()
        .arg(fixture("does-not-exist.csv"))
        .output()
        .expect("binary runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "no partial report on error");
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read input file"));
}

#[test]
fn binary_with_malformed_file_fails_with_error_message() {
    let output = binary()
        .arg(fixture("malformed.csv"))
        .output()
        .expect("binary runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "no partial report on error");
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to parse csv record"));
}
