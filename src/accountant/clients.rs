use crate::MessageType;
use crate::accountant::transactions::TxMap;
use rust_decimal::Decimal;
use std::collections::HashMap;

/// A client's account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    /// Client id.
    pub id: u16,
    /// Funds available for withdrawal (`total - held`).
    pub available: Decimal,
    /// Funds frozen by open disputes (`total - available`).
    pub held: Decimal,
    /// All funds (`available + held`).
    pub total: Decimal,
    /// Set after a chargeback; a locked account ignores every further message.
    pub locked: bool,
    /// Successful deposits and withdrawals of this client.
    pub transactions: TxMap,
}

/// All clients, keyed by client id.
pub type ClientMap = HashMap<u16, Client>;

impl Client {
    /// Creates an unlocked client with zero balances.
    #[must_use]
    pub fn new(id: u16) -> Self {
        Self {
            id,
            available: Decimal::ZERO,
            held: Decimal::ZERO,
            total: Decimal::ZERO,
            locked: false,
            transactions: TxMap::new(),
        }
    }

    /// Records a deposit. Returns `false` (and changes nothing) on overflow.
    pub(crate) fn deposit(&mut self, amount: Decimal) -> bool {
        let (Some(available), Some(total)) = (
            self.available.checked_add(amount),
            self.total.checked_add(amount),
        ) else {
            return false;
        };
        self.available = available;
        self.total = total;
        true
    }

    /// Records a withdrawal. Returns `false` (and changes nothing) when the
    /// available funds are insufficient.
    pub(crate) fn withdraw(&mut self, amount: Decimal) -> bool {
        if self.available < amount {
            return false;
        }
        self.available -= amount;
        self.total -= amount;
        true
    }

    /// Applies a dispute, resolve or chargeback to an existing transaction.
    /// Messages that violate the rules are ignored.
    pub(crate) fn adjust(&mut self, kind: MessageType, tx_id: u32) {
        let Some(tx) = self.transactions.get_mut(&tx_id) else {
            return;
        };
        let Some(amount) = tx.amount else {
            return;
        };
        match kind {
            MessageType::Dispute if !tx.dispute && !tx.chargeback => {
                tx.dispute = true;
                self.available -= amount;
                self.held += amount;
            }
            MessageType::Resolve if tx.dispute => {
                tx.dispute = false;
                self.held -= amount;
                self.available += amount;
            }
            MessageType::Chargeback if tx.dispute => {
                tx.dispute = false;
                tx.chargeback = true;
                self.held -= amount;
                self.total -= amount;
                self.locked = true;
            }
            MessageType::Dispute
            | MessageType::Resolve
            | MessageType::Chargeback
            | MessageType::Deposit
            | MessageType::Withdrawal => {}
        }
    }
}
