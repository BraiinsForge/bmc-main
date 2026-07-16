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

import IconBraiinsPool from './braiins-pool.svg';
import * as pb from '@/proto';
import { assertUnreachable } from '@/lib/ts';

export { IconBraiinsPool };

export interface AccountIconProps {
    type: pb.AccountType;
    size: number;

    className?: string;
    style?: CSSProperties;
}
export function AccountIcon(props: AccountIconProps) {
    const { type, size, style, className } = props;

    switch (type) {
        case pb.AccountType.UNSPECIFIED:
            return null;

        case pb.AccountType.BRAIINSPOOL:
            return <IconBraiinsPool width={size} style={style} className={className} />;

        default:
            assertUnreachable(type);
    }
}
