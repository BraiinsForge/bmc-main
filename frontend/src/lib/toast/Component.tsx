import { Fragment, useCallback } from 'react';
import { toast, type ToastProps } from './sdk';

// Components
import { Html } from '@/components';
import { ToastNotification } from '@carbon/react';

// Styles
import css from './Component.scss';

export const className = css.item;

/** A fully custom toast that still maintains the animations and interactions. */
export function Toast(props: ToastProps) {
    const { id, title, message, kind } = props;

    const dismiss = useCallback(() => toast.dismiss(id), [id]);

    return (
        <ToastNotification
            key={id}
            kind={kind}
            title={title}
            children={<Fragment>{typeof message === 'string' ? <Html children={message} /> : message}</Fragment>}
            onClose={dismiss}
        />
    );
}
