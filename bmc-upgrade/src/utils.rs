// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::Path;

use data_encoding::HEXUPPER;
use ring::digest::{Context, Digest, SHA256};
use tokio::{
    fs::OpenOptions,
    io::{AsyncReadExt, BufReader},
};

pub async fn file_hash(path: impl AsRef<Path>) -> std::io::Result<String> {
    let mut file = OpenOptions::new().read(true).open(path).await?;

    let mut reader = BufReader::new(&mut file);
    let digest = sha256_digest(&mut reader).await?;
    let encoded = HEXUPPER.encode(digest.as_ref());
    Ok(encoded)
}

pub async fn sha256_digest<R: AsyncReadExt + Unpin>(reader: &mut R) -> std::io::Result<Digest> {
    let mut context = Context::new(&SHA256);
    let mut buffer = [0; 1024];

    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        context.update(&buffer[..count]);
    }

    Ok(context.finish())
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn test_sha256_digest() {
        let data = b"hello world";
        let mut reader = &data[..];

        let digest = sha256_digest(&mut reader)
            .await
            .expect("BUG: failed to hash bytes");
        let encoded = HEXUPPER.encode(digest.as_ref());

        let expected_hash = {
            let mut context = Context::new(&SHA256);
            context.update(data);
            HEXUPPER.encode(context.finish().as_ref())
        };

        assert_eq!(encoded, expected_hash);
    }
}
