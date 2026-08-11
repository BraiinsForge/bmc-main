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

use std::str::FromStr as _;

use bmc_wasm_thin::ownership::{CompositorIdentity, parse_proc_stat_starttime};

#[test]
fn identity_record_round_trips() {
    let identity = CompositorIdentity {
        boot_id: "6ac9d9eb-d0de-48fe-a377-814b215af2b1".to_owned(),
        pid: 412,
        starttime: 9_876_543,
    };
    assert_eq!(
        CompositorIdentity::from_str(&identity.to_string()).expect("valid identity record"),
        identity
    );
}

#[test]
fn malformed_identity_records_are_rejected() {
    for malformed in [
        "",
        "boot 1",
        "boot pid 3",
        "boot 1 ticks",
        "boot 1 2 extra",
        "boot 0 2",
        " 1 2",
    ] {
        assert!(
            CompositorIdentity::from_str(malformed).is_err(),
            "record should be rejected: {malformed:?}"
        );
    }
}

#[test]
fn proc_stat_parser_uses_field_22_after_final_process_name_parenthesis() {
    let prefix = "123 (name with ) embedded) S";
    let fields_4_through_21 = (4..=21).map(|field| field.to_string()).collect::<Vec<_>>();
    let stat = format!("{prefix} {} 998877 23", fields_4_through_21.join(" "));
    assert_eq!(
        parse_proc_stat_starttime(&stat).expect("valid proc stat"),
        998_877
    );
}

#[test]
fn malformed_proc_stat_is_rejected() {
    for malformed in ["", "123 name S 1 2", "123 (name) S 1 2", "123 (name) S"] {
        assert!(parse_proc_stat_starttime(malformed).is_err());
    }
}
