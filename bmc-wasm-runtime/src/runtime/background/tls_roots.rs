// Copyright (C) 2026  Jan Novák
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

//! Extra root certificates for the runtime's TLS.
//!
//! Widgets reach servers on the operator's own network too, and those often
//! carry certificates from a home or company CA that Mozilla's roots don't
//! know. CA certificates (PEM) placed in [`EXTRA_CA_FILE`] are trusted beside
//! Mozilla's roots, by HTTP fetches and TLS sockets alike. Without the file
//! nothing changes.

use std::sync::OnceLock;

use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;

/// Operator-supplied CA certificates, PEM, any number.
pub(in crate::runtime) const EXTRA_CA_FILE: &str = "/etc/bmc/extra-ca.pem";

/// The extra CA certificates, read once per process; empty when the file is
/// absent or holds no usable certificate.
pub(in crate::runtime) fn extra_ca_certs() -> &'static [CertificateDer<'static>] {
    static CERTS: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();
    CERTS.get_or_init(|| {
        let pem = match std::fs::read(EXTRA_CA_FILE) {
            Ok(pem) => pem,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                tracing::warn!("extra CA certificates: cannot read {EXTRA_CA_FILE}: {e}");
                return Vec::new();
            }
        };
        let certs = parse(&pem);
        tracing::info!(
            "extra CA certificates: trusting {} from {EXTRA_CA_FILE}",
            certs.len()
        );
        certs
    })
}

/// Every certificate in `pem`; a malformed block is skipped with a warning.
fn parse(pem: &[u8]) -> Vec<CertificateDer<'static>> {
    CertificateDer::pem_slice_iter(pem)
        .filter_map(|cert| {
            cert.map_err(|e| tracing::warn!("extra CA certificates: skipping a block: {e}"))
                .ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse;

    const TEST_CA: &str = "
-----BEGIN CERTIFICATE-----
MIIBhTCCASugAwIBAgIULzh1Fbq7mC0f8dG4322Mna6LSIkwCgYIKoZIzj0EAwIw
FzEVMBMGA1UEAwwMVGVzdCBIb21lIENBMCAXDTI2MTAzMTE1MTEzNFoYDzIxMjYx
MDA3MTUxMTM0WjAXMRUwEwYDVQQDDAxUZXN0IEhvbWUgQ0EwWTATBgcqhkjOPQIB
BggqhkjOPQMBBwNCAATDMjcAJ0/sWyTSs8GLVXqDKm4ZUXeyxMFjqzifuHD517l6
fDMpcVCABkyLvMsZnpsyFMZfOGc6ur5D8HDc88gSo1MwUTAdBgNVHQ4EFgQUkn5b
szmDuRbUcWnnM3afrV92x+MwHwYDVR0jBBgwFoAUkn5bszmDuRbUcWnnM3afrV92
x+MwDwYDVR0TAQH/BAUwAwEB/zAKBggqhkjOPQQDAgNIADBFAiAV2BVQeTo4/XaW
o3o1W5sq9bp6TR9mKPm9nnwQUb4xTwIhAOjAtmFaUe6R6oiuI3/5iwG0yDx/AJCD
Z0eD9emuzY5J
-----END CERTIFICATE-----
";

    #[test]
    fn parses_each_certificate() {
        let two = format!("{TEST_CA}\n{TEST_CA}");
        assert_eq!(parse(TEST_CA.as_bytes()).len(), 1);
        assert_eq!(parse(two.as_bytes()).len(), 2);
    }

    #[test]
    fn ignores_text_without_certificates() {
        assert!(parse(b"").is_empty());
        assert!(parse(b"not a certificate\n").is_empty());
    }
}
