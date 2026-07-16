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

import { TextInput } from '@carbon/react';
import { CopyButton as Component, type CopyButtonProps } from './CopyButton';

const kinds: Array<CopyButtonProps['kind']> = [null, 'light', 'transparent', 'input-addon'];

export default {
    title: 'components/CopyButton',
    component: Component,
    args: {
        value: '12345',
        align: 'bottom',
        disabled: false,
        kind: 'transparent',
    },
    argTypes: {
        align: {
            control: { type: 'select', options: ['left', 'right', 'bottom', 'top'] },
        },
        kind: {
            control: { type: 'select', options: kinds },
        },
        disabled: {
            control: { type: 'boolean' },
        },
    },
};

const WithInput = (props: CopyButtonProps) => {
    return (
        <div style={{ position: 'relative', display: 'flex', flexDirection: 'row', width: 200 }}>
            <TextInput id={String(props.value)} value={props.value ?? undefined} labelText="" />
            <Component {...props} />
        </div>
    );
};

function getBlock(comment: string, args: CopyButtonProps, withInput?: boolean) {
    const C = withInput ? WithInput : Component;

    return (
        <div
            key={comment}
            style={{
                display: 'flex',
                flexDirection: 'column',
                gap: '1rem',
                margin: 40,
            }}
        >
            <h1 children={comment} />
            {kinds.map(kind => {
                return (
                    <div key={kind}>
                        <p children={`{ kind: ${kind} }`} />
                        <C kind={kind} title="Button Title" {...args} />
                    </div>
                );
            })}
        </div>
    );
}

export function CopyButton(args: CopyButtonProps) {
    return [getBlock('Default', args, false), getBlock('With input', args, true)];
}
