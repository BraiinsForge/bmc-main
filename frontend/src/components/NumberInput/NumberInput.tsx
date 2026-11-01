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

import type { ComponentProps } from 'react';
import { useIntl } from 'react-intl';

// biome-ignore lint/style/noRestrictedImports: the component this wraps
import { NumberInput as Upstream } from '@carbon/react';
import { assertUnreachable } from '@/lib/ts';

export type NumberInputProps = ComponentProps<typeof Upstream>;
type TranslationKey = Parameters<NonNullable<NumberInputProps['translateWithId']>>[0];

export function NumberInput(props: NumberInputProps) {
    const { formatMessage } = useIntl();
    const translateWithId = (id: TranslationKey): string => {
        switch (id) {
            case 'increment.number':
                return formatMessage({
                    defaultMessage: 'Increment number',
                    description: "Screen-reader name of the button raising a number field's value by one step",
                });
            case 'decrement.number':
                return formatMessage({
                    defaultMessage: 'Decrement number',
                    description: "Screen-reader name of the button lowering a number field's value by one step",
                });
            default:
                return assertUnreachable(id, 'number input translation id');
        }
    };
    return <Upstream translateWithId={translateWithId} {...props} />;
}
