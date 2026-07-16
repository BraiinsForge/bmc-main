// Copyright (C) 2023  Braiins Systems s.r.o.
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

use url::Url;

pub trait UrlExt {
    /// Creates a new [`Url`] with `path` adjoined to `self`. This method was created as
    /// a replacement for `Url::join()`, which has unexpected/confusing behavior.
    /// <https://github.com/servo/rust-url/issues/333>
    ///
    /// # Example
    /// ```
    /// use url::Url;
    /// use index_common::url::UrlExt;
    ///
    /// let url = Url::parse("https://braiins.com/pool").unwrap();
    /// let new_url = url.join_path("foo").unwrap().join_path("bar/baz").unwrap();
    ///
    /// assert_eq!(new_url.as_str(), "https://braiins.com/pool/foo/bar/baz")
    ///
    /// ```
    fn join_path(&self, path: impl AsRef<str>) -> Result<Url, url::ParseError>;
}

impl UrlExt for Url {
    fn join_path(&self, path: impl AsRef<str>) -> Result<Url, url::ParseError> {
        let mut url = self.clone();
        url.path_segments_mut()
            .map_err(|()| url::ParseError::RelativeUrlWithCannotBeABaseBase)?
            .pop_if_empty()
            .extend(path.as_ref().split('/'));
        Ok(url)
    }
}
