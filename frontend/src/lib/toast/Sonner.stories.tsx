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
