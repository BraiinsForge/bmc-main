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

import { Fragment, useState } from 'react';
import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';

import { Button } from '../Button';
import { ButtonGroup } from '../ButtonGroup';
import { ModalCustom as Component, type CustomModalProps as Props } from './ModalCustom';

const args = {
    id: 'id',
    title: 'Title',
    open: true,
    children: 'Modal content',

    isLoading: false,
    hideHeader: false,

    onClose: action('onClose'),
    onSubmit: action('onSubmit'),
} satisfies Props;

export default {
    title: 'components/Modals',
    component: Component,
    parameters: { controls: { exclude: /^aria-|^on*/i } },
    args,
    argTypes: {
        children: { table: { disable: true } },
        footer: { table: { disable: true } },
    },
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
                id="open-btn"
                children="Open modal"
                onClick={() => setIsOpen(true)}
                style={{ position: 'fixed', top: '50%', left: '50%', transform: 'translate(-50%, -50%)' }}
            />
            <Component
                {...args}
                open={isOpen}
                onClose={() => {
                    setIsOpen(false);
                    args.onClose?.();
                }}
                footer={
                    <div>
                        <ButtonGroup>
                            <Button id="You'll break…" kind="danger" children="You'll break…" />
                            <Button id="…like a scabbard…" kind="secondary" children="…like a scabbard…" />
                            <Button id="…with great justice" kind="primary" children="…with great justice" />
                        </ButtonGroup>
                    </div>
                }
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

export const CustomModal = (args: Props) => <Demo {...args} />;

export const CustomModalWithFocus = (args: Props) => <Demo {...args} />;
CustomModalWithFocus.args = {
    ...args,
    selectorPrimaryFocus: 'input.focus',
};
