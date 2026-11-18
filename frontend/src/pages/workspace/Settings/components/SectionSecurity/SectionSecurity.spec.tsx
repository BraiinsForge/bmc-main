// Copyright (C) 2026  Braiins Forge s.r.o.
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

import { afterEach, describe, expect, rstest, test } from '@rstest/core';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';
import { Code, ConnectError } from '@connectrpc/connect';

import { SectionSecurity, type SectionSecurityProps } from './SectionSecurity';

afterEach(cleanup);

const actions = (overrides: Partial<NonNullable<SectionSecurityProps['actions']>> = {}) => ({
    onPasswordChange: rstest.fn(),
    onPasswordRemove: rstest.fn(),
    onPasswordCreate: rstest.fn(),
    onPasswordRecheck: rstest.fn(),
    ...overrides,
});

describe('SectionSecurity', () => {
    test('a password change the page refuses says so in its open dialog', async () => {
        const refused = new ConnectError('Password is not set!', Code.FailedPrecondition);
        render(
            <IntlProvider locale="en">
                <SectionSecurity
                    hasPassword
                    actions={actions({ onPasswordChange: rstest.fn().mockRejectedValue(refused) })}
                />
            </IntlProvider>,
        );

        fireEvent.click(screen.getByRole('button', { name: 'Change Password' }));
        const dialog = await screen.findByRole('dialog', { name: 'Change Password' });
        fireEvent.change(within(dialog).getByLabelText('Current Password'), { target: { value: 'old' } });
        fireEvent.change(within(dialog).getByLabelText('New Password'), { target: { value: 'secret' } });
        fireEvent.change(within(dialog).getByLabelText('Confirm New Password'), { target: { value: 'secret' } });
        fireEvent.click(within(dialog).getByRole('button', { name: 'Change Password' }));

        expect(await within(dialog).findByRole('alertdialog', { name: 'Password is not set!' })).toBeTruthy();
    });

    test('a failed password read says so and offers to read it again', () => {
        const onPasswordRecheck = rstest.fn();
        render(
            <IntlProvider locale="en">
                <SectionSecurity hasPassword={null} actions={actions({ onPasswordRecheck })} />
            </IntlProvider>,
        );

        const error = screen.getByRole('alertdialog', { name: 'Failed to check whether a password is set!' });
        fireEvent.click(within(error).getByRole('button', { name: 'Retry' }));

        expect(onPasswordRecheck).toHaveBeenCalledOnce();
    });
});
