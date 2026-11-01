//! The accountant actor: owns all client accounts and applies the business
//! rules for deposits, withdrawals, disputes, resolves and chargebacks.

use std::time::Instant;

use crate::message::{Message, MessageType};
use crate::stash::StashTx;
use clients::{Client, ClientMap};
use transactions::Tx;

pub mod clients;
pub mod reporter;
pub mod run;
mod transactions;

/// Applies [`Message`]s to a set of clients.
///
/// Dispute-like messages referencing a transaction that does not exist yet
/// are parked in a [`StashTx`] and replayed when the transaction shows up.
#[derive(Debug, Default)]
pub struct Accountant {
    clients: ClientMap,
    stash: StashTx,
}

impl Accountant {
    /// Creates an accountant with no clients and a default [`StashTx`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an accountant with no clients and the given stash.
    #[must_use]
    pub fn with_stash(stash: StashTx) -> Self {
        Self {
            clients: ClientMap::new(),
            stash,
        }
    }

    /// Current state of all clients.
    #[must_use]
    pub fn clients(&self) -> &ClientMap {
        &self.clients
    }

    /// Messages waiting for their transaction to appear.
    #[must_use]
    pub fn stash(&self) -> &StashTx {
        &self.stash
    }

    /// Consumes the accountant and returns the clients.
    #[must_use]
    pub fn into_clients(self) -> ClientMap {
        self.clients
    }

    /// Applies `message` using the current time for stash expiry.
    pub fn apply(&mut self, message: Message) {
        self.apply_at(Instant::now(), message);
    }

    /// Applies `message`, treating `now` as the current time for stash expiry.
    pub fn apply_at(&mut self, now: Instant, message: Message) {
        match message.kind {
            MessageType::Deposit | MessageType::Withdrawal => self.apply_funds(now, message),
            MessageType::Dispute | MessageType::Resolve | MessageType::Chargeback => {
                self.apply_adjustment(now, message);
            }
        }
    }

    fn apply_funds(&mut self, now: Instant, message: Message) {
        // Malformed rows (missing, zero or negative amount) are ignored entirely
        // and must not materialize a client.
        let Some(amount) = message
            .amount
            .filter(|amount| amount.is_sign_positive() && !amount.is_zero())
        else {
            return;
        };
        let client = self
            .clients
            .entry(message.client)
            .or_insert_with(|| Client::new(message.client));
        if client.locked || client.transactions.contains_key(&message.tx) {
            return;
        }

        let succeeded = match message.kind {
            MessageType::Deposit => client.deposit(amount),
            MessageType::Withdrawal => client.withdraw(amount),
            MessageType::Dispute | MessageType::Resolve | MessageType::Chargeback => false,
        };
        if !succeeded {
            return;
        }
        client.transactions.insert(
            message.tx,
            Tx {
                id: message.tx,
                amount: Some(amount),
                dispute: false,
                chargeback: false,
            },
        );
        self.replay_stashed(now, message.client, message.tx);
    }

    fn apply_adjustment(&mut self, now: Instant, message: Message) {
        match self.clients.get_mut(&message.client) {
            Some(client) if client.transactions.contains_key(&message.tx) => {
                if !client.locked {
                    client.adjust(message.kind, message.tx);
                }
            }
            _ => self.stash.insert_at(now, message),
        }
    }

    fn replay_stashed(&mut self, now: Instant, client_id: u16, tx_id: u32) {
        let stashed = self.stash.drain_at(now, tx_id);
        let Some(client) = self.clients.get_mut(&client_id) else {
            return;
        };
        for message in stashed {
            if client.locked {
                break;
            }
            if message.client == client_id {
                client.adjust(message.kind, message.tx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accountant::run;
    use crate::message::{Message, MessageType};
    use crate::stash::StashTx;
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use std::time::{Duration, Instant};

    fn dec(value: &str) -> Decimal {
        Decimal::from_str(value).expect("valid decimal literal")
    }

    fn deposit(client: u16, tx: u32, amount: &str) -> Message {
        Message {
            kind: MessageType::Deposit,
            client,
            tx,
            amount: Some(dec(amount)),
        }
    }

    fn withdrawal(client: u16, tx: u32, amount: &str) -> Message {
        Message {
            kind: MessageType::Withdrawal,
            client,
            tx,
            amount: Some(dec(amount)),
        }
    }

    fn adjustment(kind: MessageType, client: u16, tx: u32) -> Message {
        Message {
            kind,
            client,
            tx,
            amount: None,
        }
    }

    fn dispute(client: u16, tx: u32) -> Message {
        adjustment(MessageType::Dispute, client, tx)
    }

    fn resolve(client: u16, tx: u32) -> Message {
        adjustment(MessageType::Resolve, client, tx)
    }

    fn chargeback(client: u16, tx: u32) -> Message {
        adjustment(MessageType::Chargeback, client, tx)
    }

    fn accountant_after(messages: impl IntoIterator<Item = Message>) -> Accountant {
        let mut accountant = Accountant::default();
        for message in messages {
            accountant.apply(message);
        }
        accountant
    }

    fn client(accountant: &Accountant, id: u16) -> &Client {
        accountant
            .clients()
            .get(&id)
            .unwrap_or_else(|| panic!("client {id} should exist"))
    }

    fn assert_balances(client: &Client, available: &str, held: &str, total: &str) {
        assert_eq!(client.available, dec(available), "available");
        assert_eq!(client.held, dec(held), "held");
        assert_eq!(client.total, dec(total), "total");
        assert_eq!(
            client.total,
            client.available + client.held,
            "total == available + held"
        );
    }

    // ---------------------------------------------------------------- deposit

    #[test]
    fn deposit_creates_client_and_increases_available_and_total() {
        let accountant = accountant_after([deposit(1, 1, "1.5")]);

        let client = client(&accountant, 1);
        assert_eq!(client.id, 1);
        assert_balances(client, "1.5", "0", "1.5");
        assert!(!client.locked);
    }

    #[test]
    fn deposit_records_the_transaction() {
        let accountant = accountant_after([deposit(1, 9, "2")]);

        let tx = client(&accountant, 1)
            .transactions
            .get(&9)
            .expect("tx recorded");
        assert_eq!(tx.id, 9);
        assert_eq!(tx.amount, Some(dec("2")));
        assert!(!tx.dispute);
        assert!(!tx.chargeback);
    }

    #[test]
    fn deposits_accumulate_with_four_decimal_precision() {
        let accountant = accountant_after([deposit(1, 1, "0.0001"), deposit(1, 2, "1.9999")]);

        assert_balances(client(&accountant, 1), "2.0000", "0", "2.0000");
    }

    #[test]
    fn deposit_without_amount_is_ignored() {
        let accountant = accountant_after([Message {
            kind: MessageType::Deposit,
            client: 1,
            tx: 1,
            amount: None,
        }]);

        assert!(accountant.clients().is_empty());
    }

    #[test]
    fn deposit_with_negative_or_zero_amount_is_ignored() {
        let accountant = accountant_after([deposit(1, 1, "-5"), deposit(1, 2, "0")]);

        assert!(accountant.clients().is_empty());
    }

    #[test]
    fn duplicate_transaction_id_is_ignored() {
        let accountant = accountant_after([deposit(1, 1, "1"), deposit(1, 1, "100")]);

        assert_balances(client(&accountant, 1), "1", "0", "1");
    }

    // ------------------------------------------------------------- withdrawal

    #[test]
    fn withdrawal_decreases_available_and_total() {
        let accountant = accountant_after([deposit(1, 1, "3"), withdrawal(1, 2, "1.5")]);

        assert_balances(client(&accountant, 1), "1.5", "0", "1.5");
    }

    #[test]
    fn withdrawal_of_exact_balance_leaves_zero() {
        let accountant = accountant_after([deposit(1, 1, "3"), withdrawal(1, 2, "3")]);

        assert_balances(client(&accountant, 1), "0", "0", "0");
    }

    #[test]
    fn withdrawal_with_insufficient_funds_fails_and_total_is_unchanged() {
        let accountant = accountant_after([deposit(1, 1, "2"), withdrawal(1, 2, "3")]);

        let client = client(&accountant, 1);
        assert_balances(client, "2", "0", "2");
        assert!(
            !client.transactions.contains_key(&2),
            "failed withdrawal is not recorded"
        );
    }

    #[test]
    fn withdrawal_for_unknown_client_creates_client_with_zero_balance() {
        let accountant = accountant_after([withdrawal(5, 1, "1")]);

        assert_balances(client(&accountant, 5), "0", "0", "0");
    }

    #[test]
    fn withdrawal_cannot_touch_held_funds() {
        let accountant = accountant_after([
            deposit(1, 1, "5"),
            deposit(1, 2, "1"),
            dispute(1, 1),
            withdrawal(1, 3, "2"),
        ]);

        assert_balances(client(&accountant, 1), "1", "5", "6");
    }

    // ---------------------------------------------------------------- dispute

    #[test]
    fn dispute_moves_funds_from_available_to_held() {
        let accountant = accountant_after([deposit(1, 1, "5"), deposit(1, 2, "1"), dispute(1, 1)]);

        let client = client(&accountant, 1);
        assert_balances(client, "1", "5", "6");
        assert!(client.transactions[&1].dispute);
    }

    #[test]
    fn dispute_on_withdrawal_holds_the_withdrawn_amount() {
        let accountant =
            accountant_after([deposit(1, 1, "5"), withdrawal(1, 2, "2"), dispute(1, 2)]);

        assert_balances(client(&accountant, 1), "1", "2", "3");
    }

    #[test]
    fn dispute_on_already_disputed_tx_is_ignored() {
        let accountant = accountant_after([deposit(1, 1, "5"), dispute(1, 1), dispute(1, 1)]);

        assert_balances(client(&accountant, 1), "0", "5", "5");
    }

    #[test]
    fn dispute_from_other_client_is_stashed_not_applied() {
        let accountant = accountant_after([deposit(1, 1, "5"), dispute(2, 1)]);

        assert_balances(client(&accountant, 1), "5", "0", "5");
        assert!(!accountant.clients().contains_key(&2));
        assert_eq!(accountant.stash().len(), 1);
    }

    // ---------------------------------------------------------------- resolve

    #[test]
    fn resolve_releases_held_funds_back_to_available() {
        let accountant = accountant_after([deposit(1, 1, "5"), dispute(1, 1), resolve(1, 1)]);

        let client = client(&accountant, 1);
        assert_balances(client, "5", "0", "5");
        assert!(!client.transactions[&1].dispute);
    }

    #[test]
    fn resolve_on_tx_not_under_dispute_is_ignored() {
        let accountant = accountant_after([deposit(1, 1, "5"), resolve(1, 1)]);

        assert_balances(client(&accountant, 1), "5", "0", "5");
        assert!(
            accountant.stash().is_empty(),
            "existing tx is never stashed"
        );
    }

    #[test]
    fn tx_can_be_disputed_again_after_resolve() {
        let accountant = accountant_after([
            deposit(1, 1, "5"),
            dispute(1, 1),
            resolve(1, 1),
            dispute(1, 1),
        ]);

        assert_balances(client(&accountant, 1), "0", "5", "5");
    }

    // ------------------------------------------------------------- chargeback

    #[test]
    fn chargeback_removes_held_funds_and_locks_the_account() {
        let accountant = accountant_after([
            deposit(1, 1, "5"),
            deposit(1, 2, "1"),
            dispute(1, 1),
            chargeback(1, 1),
        ]);

        let client = client(&accountant, 1);
        assert_balances(client, "1", "0", "1");
        assert!(client.locked);
        let tx = &client.transactions[&1];
        assert!(tx.chargeback);
        assert!(!tx.dispute);
    }

    #[test]
    fn chargeback_on_tx_not_under_dispute_is_ignored() {
        let accountant = accountant_after([deposit(1, 1, "5"), chargeback(1, 1)]);

        let client = client(&accountant, 1);
        assert_balances(client, "5", "0", "5");
        assert!(!client.locked);
    }

    #[test]
    fn charged_back_tx_cannot_be_disputed_again() {
        let accountant = accountant_after([
            deposit(1, 1, "5"),
            dispute(1, 1),
            chargeback(1, 1),
            dispute(1, 1),
        ]);

        assert_balances(client(&accountant, 1), "0", "0", "0");
    }

    // --------------------------------------------------------- locked account

    #[test]
    fn locked_account_ignores_deposits_and_withdrawals() {
        let accountant = accountant_after([
            deposit(1, 1, "5"),
            deposit(1, 2, "3"),
            dispute(1, 1),
            chargeback(1, 1),
            deposit(1, 3, "100"),
            withdrawal(1, 4, "1"),
        ]);

        let client = client(&accountant, 1);
        assert_balances(client, "3", "0", "3");
        assert!(!client.transactions.contains_key(&3));
        assert!(!client.transactions.contains_key(&4));
    }

    #[test]
    fn locked_account_ignores_further_disputes() {
        let accountant = accountant_after([
            deposit(1, 1, "5"),
            deposit(1, 2, "3"),
            dispute(1, 1),
            chargeback(1, 1),
            dispute(1, 2),
        ]);

        assert_balances(client(&accountant, 1), "3", "0", "3");
    }

    #[test]
    fn locking_one_client_does_not_affect_others() {
        let accountant = accountant_after([
            deposit(1, 1, "5"),
            deposit(2, 2, "7"),
            dispute(1, 1),
            chargeback(1, 1),
            deposit(2, 3, "1"),
        ]);

        assert!(client(&accountant, 1).locked);
        let other = client(&accountant, 2);
        assert!(!other.locked);
        assert_balances(other, "8", "0", "8");
    }

    // ------------------------------------------------------------------ stash

    #[test]
    fn dispute_for_unknown_client_is_stashed_and_no_client_is_created() {
        let accountant = accountant_after([dispute(1, 1)]);

        assert!(accountant.clients().is_empty());
        assert_eq!(accountant.stash().len(), 1);
    }

    #[test]
    fn dispute_for_unknown_tx_of_existing_client_is_stashed() {
        let accountant = accountant_after([deposit(1, 1, "5"), dispute(1, 2)]);

        assert_balances(client(&accountant, 1), "5", "0", "5");
        assert_eq!(accountant.stash().len(), 1);
    }

    #[test]
    fn stashed_dispute_is_applied_when_the_deposit_arrives() {
        let accountant = accountant_after([dispute(1, 1), deposit(1, 1, "5")]);

        let client = client(&accountant, 1);
        assert_balances(client, "0", "5", "5");
        assert!(client.transactions[&1].dispute);
        assert!(accountant.stash().is_empty());
    }

    #[test]
    fn stashed_dispute_is_applied_when_the_withdrawal_arrives() {
        let accountant =
            accountant_after([deposit(1, 1, "5"), dispute(1, 2), withdrawal(1, 2, "2")]);

        assert_balances(client(&accountant, 1), "1", "2", "3");
    }

    #[test]
    fn stashed_messages_replay_in_arrival_order() {
        let accountant = accountant_after([dispute(1, 1), resolve(1, 1), deposit(1, 1, "5")]);

        let client = client(&accountant, 1);
        assert_balances(client, "5", "0", "5");
        assert!(!client.transactions[&1].dispute);
    }

    #[test]
    fn stashed_resolve_before_dispute_is_ignored_on_replay() {
        let accountant = accountant_after([resolve(1, 1), dispute(1, 1), deposit(1, 1, "5")]);

        assert_balances(client(&accountant, 1), "0", "5", "5");
    }

    #[test]
    fn stashed_chargeback_replays_and_locks_the_account() {
        let accountant = accountant_after([dispute(1, 1), chargeback(1, 1), deposit(1, 1, "5")]);

        let client = client(&accountant, 1);
        assert_balances(client, "0", "0", "0");
        assert!(client.locked);
    }

    #[test]
    fn stashed_message_for_another_client_is_dropped_on_replay() {
        let accountant = accountant_after([dispute(2, 1), deposit(1, 1, "5")]);

        assert_balances(client(&accountant, 1), "5", "0", "5");
        assert!(!accountant.clients().contains_key(&2));
        assert!(accountant.stash().is_empty());
    }

    #[test]
    fn stashed_dispute_is_not_applied_when_failed_withdrawal_arrives() {
        let accountant = accountant_after([dispute(1, 2), withdrawal(1, 2, "2")]);

        assert_balances(client(&accountant, 1), "0", "0", "0");
        assert_eq!(
            accountant.stash().len(),
            1,
            "failed withdrawal creates no tx to replay on"
        );
    }

    #[test]
    fn expired_stashed_dispute_is_not_applied() {
        let start = Instant::now();
        let mut accountant = Accountant::with_stash(StashTx::new(Duration::from_secs(10)));
        accountant.apply_at(start, dispute(1, 1));

        accountant.apply_at(start + Duration::from_secs(10), deposit(1, 1, "5"));

        assert_balances(client(&accountant, 1), "5", "0", "5");
    }

    #[test]
    fn stashed_dispute_within_ttl_is_applied() {
        let start = Instant::now();
        let mut accountant = Accountant::with_stash(StashTx::new(Duration::from_secs(10)));
        accountant.apply_at(start, dispute(1, 1));

        accountant.apply_at(start + Duration::from_secs(9), deposit(1, 1, "5"));

        assert_balances(client(&accountant, 1), "0", "5", "5");
    }

    // ----------------------------------------------------------------- actor

    #[tokio::test]
    async fn run_consumes_channel_until_closed_and_returns_clients() {
        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        let actor = tokio::spawn(run::run(receiver));

        for message in [
            deposit(1, 1, "2"),
            deposit(2, 2, "3"),
            withdrawal(1, 3, "0.5"),
        ] {
            sender.send(message).await.expect("actor is alive");
        }
        drop(sender);

        let clients = actor.await.expect("actor task completes");
        assert_eq!(clients.len(), 2);
        assert_eq!(clients[&1].available, dec("1.5"));
        assert_eq!(clients[&2].available, dec("3"));
    }

    #[tokio::test]
    async fn run_with_immediately_closed_channel_returns_empty_map() {
        let (sender, receiver) = tokio::sync::mpsc::channel::<Message>(1);
        drop(sender);

        let clients = run::run(receiver).await;

        assert!(clients.is_empty());
    }

    #[test]
    fn into_clients_returns_owned_map() {
        let accountant = accountant_after([deposit(1, 1, "1")]);

        let clients: ClientMap = accountant.into_clients();

        assert_eq!(clients.len(), 1);
    }
}
