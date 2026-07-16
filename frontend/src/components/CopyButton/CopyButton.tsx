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

import type { ComponentProps } from 'react';
import copy2clipboard from 'copy-to-clipboard';

import cn from 'clsx';
import css from './CopyButton.scss';

import { CopyButton as BaseComponent } from '@carbon/react';

type BaseProps = Omit<ComponentProps<typeof BaseComponent>, keyof LocalProps | 'tooltipAlignment' | 'tooltipPosition'>;
type LocalProps = {
    id: string;
    value: Maybe<string>;
    kind?: null | 'transparent' | 'light' | 'input-addon';
};

export type CopyButtonProps = BaseProps & LocalProps;

export function CopyButton(props: CopyButtonProps) {
    const { value, kind, className, ...rest } = props;
    const disabled = value == null || value === '';

    const handleClick = () => {
        if (value) copy2clipboard(value);
    };

    const p = {
        ...rest,
        onClick: handleClick,
        disabled,
        style: { cursor: disabled ? 'not-allowed' : 'pointer', ...(rest.style || {}) },
        className: cn(css.root, kind === 'light' && css.light, kind === 'input-addon' && css.inputAddon, className),
    };
    if (disabled) p.title = undefined;
    if (kind === 'transparent') p.style.backgroundColor = 'transparent';

    return <BaseComponent {...p} />;
}
