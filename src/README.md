This program gets the csv file name as a parameter and have two tokio async actors: 1. data_stream, 2. accountant 
Program read input csv file and output csv data into the stdio, use csv crate and serde crate.

data_stream actor:
connect the file into the async-stream (use crate async-stream = "0.3.6"), reading each line in following format
`type, client, tx, amount`
The client ID will be unique per client though are not guaranteed to be ordered.
Likewise, transaction
IDs (tx) are globally unique, though are also not guaranteed to be ordered.
Whitespaces and decimal precisions
(up to four places past the decimal) must be accepted by program.
Stream is periodically sent into the accountant actor via serialized message type Message.

accountant actor: 
owns following structs

enum MessageType {
  Deposit,
  Withdrawal,
  Dispute,
  Resolve,
  Chargeback
}

struct Message {
  type: MessageType,
  client: u16,
  tx: u32,
  amount: Option<Decimal>,
}

struct Client {
	id: u16,
	available: Decimal
	held: Decimal
	total: Decimal
	locked: bool
	transactions: TxMap
}

struct ClientMap HashMap<(id, Client)>

struct Tx {
	id: u32
	amount: Option<Decimal>
	dispute: bool
	chargeeback: bool
}

struct TxMap HashMap<(id, Transaction)>

In case of message arrived, it takes the message and deserialize it into the struct Message. Then it goes thru the clients by client id `Message[client] == Client[id]`, search `Tx in Client[transactions]` where `Message[tx] == Client[transactions]Tx[id]` if client not exists, create one, if Tx not exist, create one, if both exists update by following rules:

Deposit - increase assets on clients account

Withdrawal - decrease the available and total funds on clients account. no negative charge. If a client does not have sufficient available funds the withdrawal should fail and the total amount of funds should not change

Dispute - A dispute represents a client's claim that a transaction was erroneous and should be reversed. The transaction shouldn't be reversed yet but the associated funds should be held. Clients available funds should decrease by the amount disputed, Clients held funds should increase by the amount disputed, Clients total funds should remain the same. dispute does not state the amount disputed. Instead a dispute references the transaction that is disputed by ID. If the tx specified by the dispute doesn't exist you can ignore it and assume this is an error on our partners side.

Resolve - resolution for disputed, held funds decrease, available funds increase, total should remain same. Like disputes, resolves do not specify an amount. Instead they refer to a transaction that was under dispute by ID. If the tx specified doesn't exist, or the tx isn't under dispute, you can ignore the resolve and assume this is an error on our partner's side.

Chargeback: resolution for disputed, reverse transaction. held and total should decrease. client should be locked. Like a dispute and a resolve a chargeback refers to the transaction by ID (tx) and does not specify an amount. Like a resolve, if the tx specified doesn't exist, or the tx isn't under dispute, you can ignore chargeback and assume this is an error on our partner's side.

dispute, resolve and chargeback can be applied only on existing transaction, so if we recieve those messages on Tx, Or client that does not exists, we stash it in separate ttlhashmap, let's call it StashTx, with ttl 10 minutes, so If Deposit or Withdrawal for Client comes later, we can apply dispute on it

The output is a CSV file with the following format:
List of client IDs (client), available amounts (available), held amounts (held), total amounts (total), and whether the account is locked (locked). Columns are defined as follows:

available: The total funds that are available for trading, staking, withdrawal, etc.
This should be equal to the total - held amounts

held: The total funds that are held for dispute. This should be equal to
total - available amounts

total: The total funds that are available or held. This should be equal to available
+ held

Whether the account is locked. An account is locked if a charge back occurs

For example
client, available, held, total, locked
    1, 1.5, 0.0, 1.5, false
    2, 2.0, 0.0, 2.0, false
