// Copyright (C) 2025  Braiins Systems s.r.o.

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
