//! CSV rendering of the final client balances.

use std::io::Write;

use rust_decimal::{Decimal, RoundingStrategy};
use serde::Serialize;

use crate::accountant::clients::{Client, ClientMap};

/// Number of decimal places amounts are rounded to in the report.
const REPORT_SCALE: u32 = 4;

/// One output row: `client, available, held, total, locked`.
#[derive(Debug, Serialize)]
struct ClientRecord {
    client: u16,
    available: Decimal,
    held: Decimal,
    total: Decimal,
    locked: bool,
}

impl From<&Client> for ClientRecord {
    fn from(client: &Client) -> Self {
        Self {
            client: client.id,
            available: format_amount(client.available),
            held: format_amount(client.held),
            total: format_amount(client.total),
            locked: client.locked,
        }
    }
}

/// Rounds half away from zero to four decimal places and strips trailing
/// zeros (`2.0000` -> `2`).
fn format_amount(amount: Decimal) -> Decimal {
    amount
        .round_dp_with_strategy(REPORT_SCALE, RoundingStrategy::MidpointAwayFromZero)
        .normalize()
}

/// Writes all clients as CSV to `writer`, sorted by client id.
///
/// # Errors
///
/// Returns an error when writing to `writer` fails.
pub fn write_report<W: Write>(clients: &ClientMap, writer: W) -> csv::Result<()> {
    let mut ordered: Vec<&Client> = clients.values().collect();
    ordered.sort_unstable_by_key(|client| client.id);

    let mut csv_writer = csv::Writer::from_writer(writer);
    for client in ordered {
        csv_writer.serialize(ClientRecord::from(client))?;
    }
    if clients.is_empty() {
        csv_writer.write_record(["client", "available", "held", "total", "locked"])?;
    }
    csv_writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accountant::clients::{Client, ClientMap};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn dec(value: &str) -> Decimal {
        Decimal::from_str(value).expect("valid decimal literal")
    }

    fn client(id: u16, available: &str, held: &str, locked: bool) -> Client {
        let mut client = Client::new(id);
        client.available = dec(available);
        client.held = dec(held);
        client.total = client.available + client.held;
        client.locked = locked;
        client
    }

    fn render(clients: &ClientMap) -> String {
        let mut buffer = Vec::new();
        write_report(clients, &mut buffer).expect("writing to a Vec cannot fail");
        String::from_utf8(buffer).expect("csv output is utf-8")
    }

    #[test]
    fn report_has_header_and_one_row_per_client() {
        let clients: ClientMap = [(1, client(1, "1.5", "0", false))].into_iter().collect();

        assert_eq!(
            render(&clients),
            "client,available,held,total,locked\n1,1.5,0,1.5,false\n"
        );
    }

    #[test]
    fn rows_are_sorted_by_client_id() {
        let clients: ClientMap = [
            (3, client(3, "3", "0", false)),
            (1, client(1, "1", "0", false)),
            (2, client(2, "2", "0", false)),
        ]
        .into_iter()
        .collect();

        let report = render(&clients);

        let lines: Vec<&str> = report.lines().skip(1).collect();
        assert_eq!(lines, ["1,1,0,1,false", "2,2,0,2,false", "3,3,0,3,false"]);
    }

    #[test]
    fn held_funds_and_locked_flag_are_rendered() {
        let clients: ClientMap = [(7, client(7, "1.25", "0.75", true))].into_iter().collect();

        assert_eq!(render(&clients).lines().nth(1), Some("7,1.25,0.75,2,true"));
    }

    #[test]
    fn amounts_are_rounded_to_four_decimal_places() {
        let clients: ClientMap = [(1, client(1, "1.00005", "0", false))]
            .into_iter()
            .collect();

        assert_eq!(
            render(&clients).lines().nth(1),
            Some("1,1.0001,0,1.0001,false")
        );
    }

    #[test]
    fn trailing_zeros_are_trimmed() {
        let clients: ClientMap = [(1, client(1, "2.0000", "0.0", false))]
            .into_iter()
            .collect();

        assert_eq!(render(&clients).lines().nth(1), Some("1,2,0,2,false"));
    }

    #[test]
    fn negative_available_is_rendered_as_is() {
        let clients: ClientMap = [(1, client(1, "-1.5", "3", false))].into_iter().collect();

        assert_eq!(render(&clients).lines().nth(1), Some("1,-1.5,3,1.5,false"));
    }

    #[test]
    fn empty_map_renders_header_only() {
        let clients = ClientMap::new();

        assert_eq!(render(&clients), "client,available,held,total,locked\n");
    }
}
