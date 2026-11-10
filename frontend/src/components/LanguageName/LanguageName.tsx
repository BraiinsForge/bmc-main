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

import { useIntl } from 'react-intl';
import cn from 'clsx';
import { WarningAltFilled } from '@carbon/icons-react';

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

export interface LanguageUnavailableProps {
    // Shows only the warning in the flag's place,
    // leaving the words to screen readers and the tooltip.
    flagOnly?: boolean;
}

/** Stands in for the device's language in a picker once the device no longer offers it. */
export function LanguageUnavailable({ flagOnly }: LanguageUnavailableProps) {
    const { formatMessage } = useIntl();
    const text = formatMessage({
        defaultMessage: 'Language no longer available',
        description:
            'Placeholder in a closed language dropdown, beside a warning icon, once the device no longer offers the language it is set to, such as after its language pack was removed. In the setup header only the icon shows, and the text is read out by screen readers and shown as a tooltip',
    });
    return (
        <span className={css.root} title={text}>
            <WarningAltFilled className={cn(css.flag, css.unavailable)} aria-hidden />
            <span className={flagOnly ? 'cds--visually-hidden' : undefined} children={text} />
        </span>
    );
}
