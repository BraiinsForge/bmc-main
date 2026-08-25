// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! Manual verification harness for the BOS-4004 acceptance criteria.
//!
//! ```text
//! RUST_LOG=debug cargo run -p bmc-net-mdns --example advertise -- [hostname] [suffix] [port]
//! ```
//!
//! While it runs, verify from a peer:
//!
//! ```text
//! avahi-browse -rt _http._tcp
//! avahi-browse -rt _bos._sub._http._tcp
//! ```
//!
//! Commands on stdin: `rename <hostname>` re-announces under a new name,
//! an empty line (or EOF) shuts down with goodbye packets — the instance
//! must vanish from a running `avahi-browse` within seconds.
//!
//! The name is probed only at announce time (start and rename): start a
//! second instance with the same hostname and a different suffix and watch
//! it come up under its suffixed name. A conflict arriving later is
//! deliberately not handled.

use bmc_net_mdns::{Advertisement, MdnsAdvertiser, TxtValues};
use tokio::io::AsyncBufReadExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    bmc_log::init_console();

    let mut args = std::env::args().skip(1);
    let hostname = args.next().unwrap_or_else(|| "Antminer".to_owned());
    // Only used if the hostname turns out to be taken; run two instances with
    // the same hostname and different suffixes to exercise that path.
    let conflict_suffix = args.next().unwrap_or_else(|| "3f9c2a".to_owned());
    let port = args.next().map_or(Ok(80), |p| p.parse())?;

    let mut advertiser = MdnsAdvertiser::start(Advertisement {
        hostname,
        conflict_suffix,
        port,
        txt_values: TxtValues {
            bos_version: Some("2026-07-07-0-c5a2978a-26.07-plus".to_owned()),
            bos_api_version: Some("1.6.0".to_owned()),
            miner: Some("Braiins Mini Miner BMM101".to_owned()),
        },
    })
    .await?;
    println!(
        "advertising as {}; empty line quits",
        advertiser.effective_hostname()
    );

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        match line.split_once(' ') {
            Some(("rename", hostname)) => {
                advertiser.rename(hostname).await?;
                println!("renamed to {}", advertiser.effective_hostname());
            }
            _ if line.is_empty() => break,
            _ => println!("commands: `rename <hostname>`, empty line to quit"),
        }
    }

    advertiser.shutdown().await?;
    println!("goodbye sent");
    Ok(())
}
