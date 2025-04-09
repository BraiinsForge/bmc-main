import { Fragment, useState } from 'react';
import type { Meta } from '@storybook/react';
import { action } from '@storybook/addon-actions';

import { Modal as Component, type ModalProps as Props } from './Modal';
import { Button } from '@/components';

export default {
    title: 'components/Modals',
    component: Component,
    parameters: { controls: { exclude: /^aria-|^on*/i } },
    args: {
        id: 'id',

        // Submit
        primaryButtonText: 'Submit',
        onRequestSubmit: action('onRequestSubmit'),

        // Cancel
        secondaryButtonText: 'Cancel',
        onRequestClose: action('onRequestClose'),
    } satisfies Props,
    argTypes: { children: { table: { disable: true } } },
} satisfies Meta<Props>;

function Demo(args: Props) {
    const [isOpen, setIsOpen] = useState(false);
    function getInput(className: string) {
        return (
            <input
                readOnly
                type="text"
                className={className}
                value={`input.${className}`}
                onFocus={e => e.target.select()}
                style={{
                    display: 'block',
                    width: '100%',
                    margin: '8px 0',
                    padding: '8px 16px',
                    cursor: 'default',
                    borderColor: 'transparent',
                    backgroundColor: 'rgba(255, 255, 255, 0.6)',
                }}
            />
        );
    }

    return (
        <Fragment>
            <Button
                children="Open modal"
                onClick={() => setIsOpen(true)}
                style={{ position: 'fixed', top: '50%', left: '50%', transform: 'translate(-50%, -50%)' }}
            />
            <Component
                {...args}
                open={isOpen}
                onRequestClose={e => {
                    setIsOpen(false);
                    args.onRequestClose?.(e);
                }}
            >
                <section>
                    One must love the moon in order to feel the lord of sincere sorrow.
                    {getInput('first')}
                    When one praises joy and dimension, one is able to synthesize emptiness. the follower needs!
                </section>

                <section style={{ marginTop: 16 }}>
                    Mind at the radiation dome that is when extraterrestrial queens malfunction.
                    {getInput('focus')}
                    Twisted cores lead to the pattern. Faith at the ready room was the anomaly of understanding,
                    questioned to a clear crew?
                </section>
            </Component>
        </Fragment>
    );
}

export const Modal = (args: Props) => <Demo {...args} />;
export const ModalWithFocus = (args: Props) => <Demo {...args} selectorPrimaryFocus="input.focus" />;
