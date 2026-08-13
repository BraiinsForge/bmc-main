// Copyright (C) 2025  Braiins Systems s.r.o.
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

use std::io::{self, Write};
use zip::unstable::write::FileOptionsExt;
use zip::write::SimpleFileOptions;

/// A [`Write`] with a [`finish`](Self::finish) step that completes the stream.
pub trait FinishWrite: Write {
    fn finish(self: Box<Self>) -> io::Result<()>;
}

/// Stream encoding of the support archive.
///
/// A format can wrap the raw output stream (whole-stream encryption) and/or
/// adjust the per-file zip entry options (per-entry encryption). Consumer
/// binaries implement this to add encodings the crate does not ship — a
/// pass-through `wrap` can delegate to [`PlainZip::wrap`].
pub trait ArchiveFormat: Sync {
    /// Wrap the raw output stream the zip data is written to.
    fn wrap<'w>(&self, writer: Box<dyn Write + 'w>) -> Box<dyn FinishWrite + 'w>;

    /// Adjust the per-file zip entry options.
    fn file_options(&self, options: SimpleFileOptions) -> SimpleFileOptions {
        options
    }
}

/// Plain unencrypted zip stream.
#[derive(Debug)]
pub struct PlainZip;

impl ArchiveFormat for PlainZip {
    fn wrap<'w>(&self, writer: Box<dyn Write + 'w>) -> Box<dyn FinishWrite + 'w> {
        Box::new(PlainWriter(writer))
    }
}

struct PlainWriter<'w>(Box<dyn Write + 'w>);

impl Write for PlainWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl FinishWrite for PlainWriter<'_> {
    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.0.flush()
    }
}

/// Password the support archive entries are encrypted under. Fixed and
/// public; this is obfuscation for the support workflow, not user-controlled
/// secrecy — the credential censoring step is what protects secrets.
const SUPPORT_ARCHIVE_PASSWORD: &[u8] = b"braiins";

/// Zip whose entries are encrypted with the legacy ZipCrypto cipher under a
/// fixed password — the password-protected support archive format.
#[derive(Debug)]
pub struct PasswordProtectedZip;

impl ArchiveFormat for PasswordProtectedZip {
    fn wrap<'w>(&self, writer: Box<dyn Write + 'w>) -> Box<dyn FinishWrite + 'w> {
        PlainZip.wrap(writer)
    }

    fn file_options(&self, options: SimpleFileOptions) -> SimpleFileOptions {
        options
            .with_deprecated_encryption(SUPPORT_ARCHIVE_PASSWORD)
            .expect("BUG: failed to enable zip encryption")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    fn write_through_format(
        format: &dyn ArchiveFormat,
        input: &[u8],
        chunk_size: usize,
    ) -> Vec<u8> {
        let mut output = Vec::new();
        let mut writer = format.wrap(Box::new(&mut output));
        for chunk in input.chunks(chunk_size) {
            writer.write_all(chunk).expect("BUG: write failed");
        }
        writer.finish().expect("BUG: finish failed");
        output
    }

    fn zip_through_format(format: &dyn ArchiveFormat) -> Vec<u8> {
        let mut output = Vec::new();
        let stream = format.wrap(Box::new(&mut output));
        let mut zip = zip::ZipWriter::new_stream(stream);
        let options = format.file_options(SimpleFileOptions::default());
        zip.start_file("test/hello.txt", options)
            .expect("BUG: start_file failed");
        zip.write_all(b"hello world").expect("BUG: write failed");
        let stream = zip.finish().expect("BUG: zip finish failed");
        stream
            .into_inner()
            .finish()
            .expect("BUG: stream finish failed");
        output
    }

    #[test]
    fn plain_wrap_passes_bytes_through() {
        let input = b"pass-through data";
        assert_eq!(write_through_format(&PlainZip, input, 5), input);
    }

    #[test]
    fn zip_over_plain_format_is_valid_zip() {
        let data = zip_through_format(&PlainZip);
        let mut zip = ZipArchive::new(Cursor::new(&data)).expect("BUG: should be a valid zip");
        let mut content = String::new();
        zip.by_name("test/hello.txt")
            .expect("BUG: entry not found")
            .read_to_string(&mut content)
            .expect("BUG: read failed");
        assert_eq!(content, "hello world");
    }

    #[test]
    fn password_protected_format_encrypts_entries_of_a_valid_zip() {
        let data = zip_through_format(&PasswordProtectedZip);
        assert!(
            !data.windows(11).any(|w| w == b"hello world"),
            "entry content must not appear in plaintext"
        );
        let mut zip = ZipArchive::new(Cursor::new(&data)).expect("BUG: should be a valid zip");
        let mut content = String::new();
        zip.by_name_decrypt("test/hello.txt", SUPPORT_ARCHIVE_PASSWORD)
            .expect("BUG: decrypt failed")
            .read_to_string(&mut content)
            .expect("BUG: read failed");
        assert_eq!(content, "hello world");
    }
}
