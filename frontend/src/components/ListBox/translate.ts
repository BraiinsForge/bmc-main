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

import type { IntlShape } from 'react-intl';
import type { ListBoxMenuIconTranslationKey, ListBoxSelectionTranslationKey } from '@carbon/react';
import { assertUnreachable } from '@/lib/ts';

type TranslationKey = ListBoxMenuIconTranslationKey | ListBoxSelectionTranslationKey;

/** Carbon's `translateWithId` for a dropdown's or a combo box's buttons. */
export function listBoxTranslate(formatMessage: IntlShape['formatMessage']): (id: TranslationKey) => string {
    return id => {
        switch (id) {
            case 'open.menu':
                return formatMessage({
                    defaultMessage: 'Open',
                    description: "Screen-reader name of a dropdown's arrow button while its list is closed",
                });
            case 'close.menu':
                return formatMessage({
                    defaultMessage: 'Close',
                    description: "Screen-reader name of a dropdown's arrow button while its list is open",
                });
            case 'clear.selection':
                return formatMessage({
                    defaultMessage: 'Clear selected item',
                    description: "Screen-reader name of the button emptying a dropdown's choice",
                });
            case 'clear.all':
                return formatMessage({
                    defaultMessage: 'Clear selected items',
                    description: "Screen-reader name of the button emptying a multi-choice dropdown's choices",
                });
            default:
                return assertUnreachable(id, 'list box translation id');
        }
    };
}
