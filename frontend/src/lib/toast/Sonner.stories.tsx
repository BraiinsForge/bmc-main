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

import { useState, useCallback, type FormEvent, Fragment } from 'react';
import type { ArgTypes } from '@storybook/react';

import * as gen from '@/mocks';
import { Button } from '@/components';
import { TextInput } from '@carbon/react';

import { toast, Toaster, type ToastProps, type ToasterProps } from '@/lib/toast';

export default { title: 'components/Toast' };

function getRandomKind(): ToastProps['kind'] {
    return gen.randomItem(['success', 'info', 'warning', 'error'] as const);
}

function TriggerToast() {
    const [title, setTitle] = useState('This is a title');
    const [content, setContent] = useState('This is a content');
    const handleSubmit = useCallback(
        (e: FormEvent) => {
            e.preventDefault();
            toast.show(getRandomKind(), content, { title });
        },
        [title, content],
    );

    return (
        <form
            onSubmit={handleSubmit}
            style={{
                display: 'flex',
                flexDirection: 'column',
                gap: '1rem',
                marginTop: '1rem',
                padding: '1rem',
                maxWidth: '300px',
            }}
        >
            <TextInput labelText="Title" id="title" value={title} onChange={e => setTitle(e.target.value)} />
            <TextInput labelText="Content" id="content" value={content} onChange={e => setContent(e.target.value)} />
            <Button id="render-toast" type="submit" children="Render toast" />
        </form>
    );
}

const argTypes = {
    duration: { control: { type: 'number' } },
    visibleToasts: { control: { type: 'number', defaultValue: 3 } },
    position: {
        control: { type: 'select', defaultValue: 'bottom-right' },
        options: ['top-left', 'top-right', 'bottom-left', 'bottom-right'],
    },
} satisfies ArgTypes<ToasterProps>;

export const Stacked = {
    render: (args: ToasterProps) => (
        <Fragment>
            <Toaster {...args} />
            <TriggerToast />
        </Fragment>
    ),
    argTypes,
};

export const Expanded = {
    render: (args: ToasterProps) => (
        <Fragment>
            <Toaster {...args} expand={true} />
            <TriggerToast />
        </Fragment>
    ),
    argTypes,
};

export const Infinite = {
    render: (args: ToasterProps) => (
        <Fragment>
            <Toaster {...args} duration={Number.POSITIVE_INFINITY} />
            <TriggerToast />
        </Fragment>
    ),
    argTypes,
};
