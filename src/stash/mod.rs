//! Temporary storage for dispute-like messages that arrived before the
//! transaction they refer to.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::message::Message;

/// A stashed message together with the moment it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stashed {
    stored_at: Instant,
    message: Message,
}

/// Time-limited stash of dispute, resolve and chargeback messages whose
/// transaction (or client) did not exist yet when they arrived.
///
/// Entries are keyed by transaction id and kept in arrival order. An entry
/// expires once it has been stored for at least the configured TTL; expired
/// entries are dropped lazily on [`insert_at`](Self::insert_at),
/// [`drain_at`](Self::drain_at) and [`purge_expired`](Self::purge_expired).
///
/// The current time is always passed in explicitly so the behaviour is
/// deterministic and testable; callers normally pass [`Instant::now`].
#[derive(Debug, Clone)]
pub struct StashTx {
    ttl: Duration,
    entries: HashMap<u32, Vec<Stashed>>,
}

impl StashTx {
    /// TTL used by [`StashTx::default`]: ten minutes.
    pub const DEFAULT_TTL: Duration = Duration::from_secs(10 * 60);

    /// Creates an empty stash whose entries expire after `ttl`.
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: HashMap::new(),
        }
    }

    /// Time an entry is retained before it expires.
    #[must_use]
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Number of stashed messages (across all transaction ids).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }

    /// `true` when nothing is stashed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Stores `message` under its transaction id at time `now`, dropping any
    /// entries that have already expired.
    pub fn insert_at(&mut self, now: Instant, message: Message) {
        self.purge_expired(now);
        self.entries.entry(message.tx).or_default().push(Stashed {
            stored_at: now,
            message,
        });
    }

    /// Removes and returns every non-expired message stashed for `tx`, in
    /// arrival order. Expired messages for `tx` are discarded.
    #[must_use]
    pub fn drain_at(&mut self, now: Instant, tx: u32) -> Vec<Message> {
        let Some(stashed) = self.entries.remove(&tx) else {
            return Vec::new();
        };
        stashed
            .into_iter()
            .filter(|entry| !self.is_expired(now, entry))
            .map(|entry| entry.message)
            .collect()
    }

    /// Drops every entry that has been stored for at least the TTL.
    pub fn purge_expired(&mut self, now: Instant) {
        let ttl = self.ttl;
        self.entries.retain(|_, stashed| {
            stashed.retain(|entry| now.saturating_duration_since(entry.stored_at) < ttl);
            !stashed.is_empty()
        });
    }

    fn is_expired(&self, now: Instant, entry: &Stashed) -> bool {
        now.saturating_duration_since(entry.stored_at) >= self.ttl
    }
}

impl Default for StashTx {
    fn default() -> Self {
        Self::new(Self::DEFAULT_TTL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, MessageType};
    use std::time::{Duration, Instant};

    fn adjustment(kind: MessageType, client: u16, tx: u32) -> Message {
        Message {
            kind,
            client,
            tx,
            amount: None,
        }
    }

    #[test]
    fn default_ttl_is_ten_minutes() {
        assert_eq!(StashTx::DEFAULT_TTL, Duration::from_secs(10 * 60));
        assert_eq!(StashTx::default().ttl(), StashTx::DEFAULT_TTL);
    }

    #[test]
    fn stashed_message_is_returned_when_drained_before_ttl() {
        let now = Instant::now();
        let mut stash = StashTx::default();
        stash.insert_at(now, adjustment(MessageType::Dispute, 1, 7));

        let drained = stash.drain_at(now + Duration::from_secs(60), 7);

        assert_eq!(drained, vec![adjustment(MessageType::Dispute, 1, 7)]);
        assert!(stash.is_empty());
    }

    #[test]
    fn drain_preserves_arrival_order_for_the_same_tx() {
        let now = Instant::now();
        let mut stash = StashTx::default();
        stash.insert_at(now, adjustment(MessageType::Dispute, 1, 7));
        stash.insert_at(
            now + Duration::from_secs(1),
            adjustment(MessageType::Resolve, 1, 7),
        );

        let drained = stash.drain_at(now + Duration::from_secs(2), 7);

        assert_eq!(
            drained,
            vec![
                adjustment(MessageType::Dispute, 1, 7),
                adjustment(MessageType::Resolve, 1, 7),
            ]
        );
    }

    #[test]
    fn drain_only_returns_messages_for_the_requested_tx() {
        let now = Instant::now();
        let mut stash = StashTx::default();
        stash.insert_at(now, adjustment(MessageType::Dispute, 1, 7));
        stash.insert_at(now, adjustment(MessageType::Dispute, 2, 8));

        let drained = stash.drain_at(now, 7);

        assert_eq!(drained, vec![adjustment(MessageType::Dispute, 1, 7)]);
        assert_eq!(stash.len(), 1);
    }

    #[test]
    fn drain_of_unknown_tx_returns_nothing() {
        let mut stash = StashTx::default();

        let drained = stash.drain_at(Instant::now(), 42);

        assert!(drained.is_empty());
    }

    #[test]
    fn expired_message_is_not_returned_by_drain() {
        let now = Instant::now();
        let mut stash = StashTx::new(Duration::from_secs(10));
        stash.insert_at(now, adjustment(MessageType::Dispute, 1, 7));

        let drained = stash.drain_at(now + Duration::from_secs(10), 7);

        assert!(drained.is_empty(), "message exactly at ttl must be expired");
        assert!(stash.is_empty());
    }

    #[test]
    fn drain_keeps_fresh_messages_and_drops_expired_ones_for_the_same_tx() {
        let now = Instant::now();
        let mut stash = StashTx::new(Duration::from_secs(10));
        stash.insert_at(now, adjustment(MessageType::Dispute, 1, 7));
        stash.insert_at(
            now + Duration::from_secs(5),
            adjustment(MessageType::Resolve, 1, 7),
        );

        let drained = stash.drain_at(now + Duration::from_secs(12), 7);

        assert_eq!(drained, vec![adjustment(MessageType::Resolve, 1, 7)]);
    }

    #[test]
    fn insert_purges_expired_entries_of_other_txs() {
        let now = Instant::now();
        let mut stash = StashTx::new(Duration::from_secs(10));
        stash.insert_at(now, adjustment(MessageType::Dispute, 1, 7));

        stash.insert_at(
            now + Duration::from_secs(11),
            adjustment(MessageType::Dispute, 2, 8),
        );

        assert_eq!(stash.len(), 1);
        assert!(stash.drain_at(now + Duration::from_secs(11), 7).is_empty());
    }

    #[test]
    fn purge_expired_removes_only_stale_entries() {
        let now = Instant::now();
        let mut stash = StashTx::new(Duration::from_secs(10));
        stash.insert_at(now, adjustment(MessageType::Dispute, 1, 7));
        stash.insert_at(
            now + Duration::from_secs(8),
            adjustment(MessageType::Dispute, 2, 8),
        );

        stash.purge_expired(now + Duration::from_secs(10));

        assert_eq!(stash.len(), 1);
        assert_eq!(
            stash.drain_at(now + Duration::from_secs(10), 8),
            vec![adjustment(MessageType::Dispute, 2, 8)]
        );
    }

    #[test]
    fn time_going_backwards_does_not_panic_or_expire() {
        let now = Instant::now();
        let mut stash = StashTx::new(Duration::from_secs(10));
        stash.insert_at(
            now + Duration::from_secs(5),
            adjustment(MessageType::Dispute, 1, 7),
        );

        let drained = stash.drain_at(now, 7);

        assert_eq!(drained.len(), 1);
    }
}
