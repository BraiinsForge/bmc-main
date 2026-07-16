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

import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Content } from '@carbon/react';

import css from './LayoutPlain.scss';

export interface LayoutPlainProps {
    children: ReactNode;
}
interface Props extends LayoutPlainProps {
    intl: IntlShape;
}

class Base extends Component<Props> {
    render() {
        const { children } = this.props;

        return <Content id="main-content" className={css.content} children={children} />;
    }
}

export function LayoutPlain(props: LayoutPlainProps) {
    const intl = useIntl();
    return <Base {...props} intl={intl} />;
}
