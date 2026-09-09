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

import { useIntl } from 'react-intl';
import type { ReactElement } from 'react';

import { InlineNotification } from '@/components';

export interface CapacityWarningProps {
    count: number;
    max: number;
    className?: string;
}

export function CapacityWarning(props: CapacityWarningProps): null | ReactElement {
    const { count, max, className } = props;
    const { formatMessage } = useIntl();

    // A missing `max` arrives as the proto default, and an unknown limit
    // is not a reached one — say nothing rather than claim falsity.
    if (max <= 0 || count < max) return null;

    return (
        <InlineNotification
            className={className}
            kind="warning"
            theme="inverse"
            stretch
            hideCloseButton
            title={formatMessage({ defaultMessage: 'All widget slots are in use' })}
            children={formatMessage(
                {
                    defaultMessage:
                        'This Deck runs up to {max} widgets at once. Turn off or delete a scene to free a slot before adding another.',
                },
                { max },
            )}
        />
    );
}
