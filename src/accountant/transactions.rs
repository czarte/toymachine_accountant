use rust_decimal::Decimal;
use std::collections::HashMap;

/// A deposit or withdrawal recorded on a client's account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tx {
    /// Globally unique transaction id.
    pub id: u32,
    /// Amount moved by the transaction.
    pub amount: Option<Decimal>,
    /// `true` while the transaction is under dispute.
    pub dispute: bool,
    /// `true` once the transaction has been reversed by a chargeback.
    pub chargeback: bool,
}

/// Transactions of one client, keyed by transaction id.
pub type TxMap = HashMap<u32, Tx>;
