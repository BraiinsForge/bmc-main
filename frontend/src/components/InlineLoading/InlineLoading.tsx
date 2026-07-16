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

import { InlineLoading as Upstream, type InlineLoadingProps as UpstreamProps } from '@carbon/react';
import cn from 'clsx';

export type InlineLoadingProps = {
    description?: UpstreamProps['description'];
    iconDescription?: UpstreamProps['iconDescription'];
    status: NonNullable<UpstreamProps['status']> | ReactElement;

    className?: string;
    style?: CSSProperties;
};

export function InlineLoading({ status, iconDescription, description, className, style }: InlineLoadingProps) {
    if (typeof status === 'string') {
        return <Upstream status={status} iconDescription={iconDescription} description={description} />;
    }
    return (
        <div aria-live="assertive" className={cn('cds--inline-loading', className)} style={style}>
            <div className="cds--inline-loading__animation" children={status} />
            <div className="cds--inline-loading__text" children={description} />
        </div>
    );
}
