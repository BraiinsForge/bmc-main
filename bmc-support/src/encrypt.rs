// Copyright (C) 2025  Braiins Systems s.r.o.
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

use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};

// corresponds to password "braiins"
const AES128_KEY: [u8; 16] = [
    18, 236, 21, 126, 23, 171, 61, 138, 120, 21, 42, 210, 145, 74, 59, 153,
];
const AES128_IV: [u8; 16] = [
    38, 120, 45, 252, 31, 135, 27, 211, 89, 168, 196, 56, 227, 3, 208, 152,
];

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

/// This is a trivial single-purpose symmetric encryption with a fixed password that is used for file content
/// obfuscation. The encrypted file can be decrypted using this command:
/// `openssl aes-128-cbc -d -md md5 -nosalt -pass pass:braiins -in input.zip.enc -out output.zip`
#[must_use]
pub fn encrypt(buffer: &[u8]) -> Vec<u8> {
    Aes128CbcEnc::new(&AES128_KEY.into(), &AES128_IV.into()).encrypt_padded_vec_mut::<Pkcs7>(buffer)
}
