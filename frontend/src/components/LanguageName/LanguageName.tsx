// Copyright (C) 2026  Braiins Systems s.r.o.
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

import type * as pb from '@/proto';

import css from './LanguageName.scss';

export interface LanguageNameProps {
    language: pb.LanguageInfo;
    /** Shows only the flag when there is one, leaving the name to screen readers. */
    flagOnly?: boolean;
}

/** A language's flag, when its pack ships one, followed by its native name. */
export function LanguageName({ language, flagOnly }: LanguageNameProps) {
    const { flag } = language;
    return (
        <span className={css.root} lang={language.tag}>
            {flag ? <img className={css.flag} src={`data:${flag.mimeType};base64,${flag.data}`} alt="" /> : null}
            <span className={flagOnly && flag ? 'cds--visually-hidden' : undefined} children={language.nativeName} />
        </span>
    );
}
