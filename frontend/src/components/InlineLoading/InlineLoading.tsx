// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { useIntl, type IntlShape } from 'react-intl';

// biome-ignore lint/style/noRestrictedImports: the component this wraps
import { InlineLoading as Upstream, type InlineLoadingProps as UpstreamProps } from '@carbon/react';
import { assertUnreachable } from '@/lib/ts';
import cn from 'clsx';

export type InlineLoadingProps = {
    description?: UpstreamProps['description'];
    iconDescription?: UpstreamProps['iconDescription'];
    status: NonNullable<UpstreamProps['status']> | ReactElement;

    className?: string;
    style?: CSSProperties;
};

export function InlineLoading({ status, iconDescription, description, className, style }: InlineLoadingProps) {
    const { formatMessage } = useIntl();
    if (typeof status === 'string') {
        return (
            <Upstream
                status={status}
                iconDescription={iconDescription ?? statusIconDescription(formatMessage, status)}
                description={description}
            />
        );
    }
    return (
        <div aria-live="assertive" className={cn('cds--inline-loading', className)} style={style}>
            <div className="cds--inline-loading__animation" children={status} />
            <div className="cds--inline-loading__text" children={description} />
        </div>
    );
}

function statusIconDescription(
    formatMessage: IntlShape['formatMessage'],
    status: NonNullable<UpstreamProps['status']>,
): string {
    switch (status) {
        case 'active':
        case 'inactive':
            return formatMessage({
                defaultMessage: 'Loading',
                description: 'Screen-reader name of a busy spinner',
            });
        case 'finished':
            return formatMessage({
                defaultMessage: 'Done',
                description: "Screen-reader name of the check mark replacing a busy spinner once it's done",
            });
        case 'error':
            return formatMessage({
                defaultMessage: 'Failed',
                description: 'Screen-reader name of the mark replacing a busy spinner when it failed',
            });
        default:
            return assertUnreachable(status, 'inline loading status');
    }
}
