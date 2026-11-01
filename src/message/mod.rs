//! Wire format of a single transaction row.

use rust_decimal::Decimal;
use serde::Deserialize;

/// Kind of transaction carried by a [`Message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageType {
    /// Credit funds to a client's account.
    Deposit,
    /// Debit funds from a client's account.
    Withdrawal,
    /// Claim that a previous transaction was erroneous; holds its funds.
    Dispute,
    /// Settle a dispute in the client's favour; releases held funds.
    Resolve,
    /// Settle a dispute against the client; reverses the transaction and locks the account.
    Chargeback,
}

/// One row of the input CSV: `type, client, tx, amount`.
///
/// `amount` is present for deposits and withdrawals and absent for the
/// dispute-like messages, which only reference a transaction by id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Message {
    /// Transaction kind (CSV column `type`).
    #[serde(rename = "type")]
    pub kind: MessageType,
    /// Client the transaction belongs to.
    pub client: u16,
    /// Globally unique transaction id.
    pub tx: u32,
    /// Transaction amount with up to four decimal places.
    #[serde(default)]
    pub amount: Option<Decimal>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use csv::{ReaderBuilder, Trim};
    use rust_decimal::Decimal;

    /// Mirrors the reader configuration used by `data_stream`.
    fn parse(csv: &str) -> Vec<csv::Result<Message>> {
        ReaderBuilder::new()
            .trim(Trim::All)
            .flexible(true)
            .from_reader(csv.as_bytes())
            .deserialize()
            .collect()
    }

    fn parse_one(csv: &str) -> Message {
        let mut rows = parse(csv);
        assert_eq!(rows.len(), 1, "expected exactly one row");
        rows.remove(0).expect("row should deserialize")
    }

    #[test]
    fn deposit_row_deserializes_with_amount() {
        let message = parse_one("type,client,tx,amount\ndeposit,1,7,1.5\n");

        assert_eq!(message.kind, MessageType::Deposit);
        assert_eq!(message.client, 1);
        assert_eq!(message.tx, 7);
        assert_eq!(message.amount, Some(Decimal::new(15, 1)));
    }

    #[test]
    fn every_message_type_is_recognised_in_lowercase() {
        let csv = "type,client,tx,amount\n\
                   deposit,1,1,1.0\n\
                   withdrawal,1,2,1.0\n\
                   dispute,1,1,\n\
                   resolve,1,1,\n\
                   chargeback,1,1,\n";

        let kinds: Vec<MessageType> = parse(csv)
            .into_iter()
            .map(|row| row.expect("row should deserialize").kind)
            .collect();

        assert_eq!(
            kinds,
            [
                MessageType::Deposit,
                MessageType::Withdrawal,
                MessageType::Dispute,
                MessageType::Resolve,
                MessageType::Chargeback,
            ]
        );
    }

    #[test]
    fn whitespace_around_fields_is_accepted() {
        let message = parse_one("type, client, tx, amount\n  withdrawal ,  2 , 10 ,  2.25 \n");

        assert_eq!(message.kind, MessageType::Withdrawal);
        assert_eq!(message.client, 2);
        assert_eq!(message.tx, 10);
        assert_eq!(message.amount, Some(Decimal::new(225, 2)));
    }

    #[test]
    fn four_decimal_places_are_preserved_exactly() {
        let message = parse_one("type,client,tx,amount\ndeposit,1,1,0.0001\n");

        assert_eq!(message.amount, Some(Decimal::new(1, 4)));
    }

    #[test]
    fn empty_amount_is_none() {
        let message = parse_one("type,client,tx,amount\ndispute,1,1,\n");

        assert_eq!(message.amount, None);
    }

    #[test]
    fn missing_trailing_amount_column_is_none() {
        let message = parse_one("type,client,tx,amount\ndispute,1,1\n");

        assert_eq!(message.kind, MessageType::Dispute);
        assert_eq!(message.amount, None);
    }

    #[test]
    fn unknown_type_is_an_error() {
        let rows = parse("type,client,tx,amount\nrefund,1,1,1.0\n");

        assert!(rows[0].is_err());
    }

    #[test]
    fn non_numeric_amount_is_an_error() {
        let rows = parse("type,client,tx,amount\ndeposit,1,1,abc\n");

        assert!(rows[0].is_err());
    }

    #[test]
    fn client_id_out_of_u16_range_is_an_error() {
        let rows = parse("type,client,tx,amount\ndeposit,70000,1,1.0\n");

        assert!(rows[0].is_err());
    }
}
