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

//! Reading an asset file for the asset macros.
//!
//! A machine without git-lfs checks out a short text pointer in each asset's place.
//! Embedded as is, it surfaces far from its cause, as a decode failure on the device,
//! so every LFS-tracked file the macros embed passes [`refuse_pointer`], most of them through [`read`].

use std::path::Path;

use bmc_shared_utils::asset::is_lfs_pointer;
use proc_macro2::Span;

pub fn read(path: &Path, span: Span) -> syn::Result<Vec<u8>> {
    let bytes = std::fs::read(path).map_err(|error| {
        syn::Error::new(
            span,
            format!("failed to read `{}`: {error}", path.display()),
        )
    })?;
    refuse_pointer(path, &bytes, span)?;
    Ok(bytes)
}

pub fn refuse_pointer(path: &Path, bytes: &[u8], span: Span) -> syn::Result<()> {
    if is_lfs_pointer(bytes) {
        return Err(syn::Error::new(
            span,
            format!(
                "`{}` is a git-lfs pointer stub, not the file; install git-lfs and run `git lfs pull`",
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use proc_macro2::Span;

    use super::read;

    const LFS_POINTER: &str = "version https://git-lfs.github.com/spec/v1\n\
                               oid sha256:72d28658ad6339e7c5b800047c40de69576e4f22cea45929646f60af56d9c027\n\
                               size 6767\n";

    fn file_with(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("BUG: the temp dir is writable");
        file.write_all(bytes)
            .expect("BUG: the temp file is writable");
        file
    }

    #[test]
    fn refuses_a_pointer_naming_the_file_and_the_fix() {
        let pointer = file_with(LFS_POINTER.as_bytes());

        let message = read(pointer.path(), Span::call_site())
            .expect_err("a pointer must not be embedded as the asset")
            .to_string();

        let path = pointer.path().display().to_string();
        assert!(
            message.contains(&path),
            "the error names the file to fetch: {message}"
        );
        assert!(
            message.contains("git lfs pull"),
            "the error names the fix: {message}"
        );
    }

    #[test]
    fn returns_real_asset_bytes() {
        let png_head: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        let wav_head: &[u8] = b"RIFF\x24\0\0\0WAVEfmt ";

        for bytes in [png_head, wav_head] {
            let asset = file_with(bytes);
            let read_bytes =
                read(asset.path(), Span::call_site()).expect("an asset is not a pointer");
            assert_eq!(read_bytes, bytes, "the macro embeds the file as read");
        }
    }
}
