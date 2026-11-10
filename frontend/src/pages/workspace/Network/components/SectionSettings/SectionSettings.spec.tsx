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
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';

import * as pb from '@/proto';

import { SectionSettings } from './SectionSettings';

const SECURED_NETWORK = pb.create(pb.WifiNetworkSchema, {
    ssid: 'HomeNet',
    encryptionType: pb.EncryptionType.WPA2,
});

function renderSettings(join: () => Promise<boolean> = async () => true) {
    const onConnectionRequest = rstest.fn(join);
    const onConnectionRequestCancel = rstest.fn();
    render(
        <IntlProvider locale="en">
            <SectionSettings
                status={[]}
                hostname={null}
                hasUnsavedChanges={false}
                onSave={() => {}}
                onReset={() => {}}
                wifi={{
                    strings: { connect: 'Connect' },
                    activeNetwork: { value: null, onConnectionRequest, onConnectionRequestCancel },
                    availableNetworks: { isLoading: false, options: [SECURED_NETWORK], onRefresh: () => {} },
                }}
            />
        </IntlProvider>,
    );
    return { onConnectionRequest, onConnectionRequestCancel };
}

const dropdown = () => screen.getByRole('combobox', { name: 'Network' });

// The password dialog's title is markup, so the dialog has no accessible name of its own
function dialogTitled(title: string): HTMLElement {
    const dialog = screen.getByRole('heading', { name: title }).closest('[role="dialog"]');
    if (!(dialog instanceof HTMLElement)) throw new Error(`BUG: the heading ${title} sits outside a dialog`);
    return dialog;
}

async function chooseNetwork(name: string): Promise<void> {
    await act(async () => {
        fireEvent.click(screen.getByRole('combobox', { name: 'Network' }));
    });
    await act(async () => {
        fireEvent.click(screen.getByRole('option', { name }));
    });
}

const manualInput = (id: 'ssid' | 'security' | 'password') => {
    const input = document.getElementById(`wifi-manual-${id}`);
    if (input == null) throw new Error(`BUG: the manual dialog has no ${id} input`);
    return input;
};

const passwordInput = () => {
    const input = document.getElementById('wifi-password-entry');
    if (input == null) throw new Error('BUG: the password dialog has no password input');
    return input;
};

const errorUnder = (inputId: string) => document.getElementById(`${inputId}-error-msg`)?.textContent;

async function changeInput(input: HTMLElement, value: string | number): Promise<void> {
    await act(async () => {
        fireEvent.change(input, { target: { value } });
    });
}

async function clickButton(name: string): Promise<void> {
    await act(async () => {
        fireEvent.click(screen.getByRole('button', { name }));
    });
}

describe('<SectionSettings /> joining a network by hand', () => {
    afterEach(cleanup);

    test('an empty SSID is refused under its field', async () => {
        const { onConnectionRequest } = renderSettings();
        await chooseNetwork('Other…');

        await clickButton('Join');

        expect(errorUnder('wifi-manual-ssid')).toBe('Missing value!');
        expect(onConnectionRequest).not.toHaveBeenCalled();
    });

    test('an open network joins without a password', async () => {
        const { onConnectionRequest } = renderSettings();
        await chooseNetwork('Other…');
        await changeInput(manualInput('ssid'), 'OpenNet');

        await clickButton('Join');

        expect(onConnectionRequest).toHaveBeenCalledWith('OpenNet', pb.EncryptionType.NONE, '');
    });

    test('a secured network without a password is refused under its field', async () => {
        const { onConnectionRequest } = renderSettings();
        await chooseNetwork('Other…');
        await changeInput(manualInput('ssid'), 'HiddenNet');
        await changeInput(manualInput('security'), pb.EncryptionType.WPA2);

        await clickButton('Join');

        expect(errorUnder('wifi-manual-password')).toBe('Missing value!');
        expect(errorUnder('wifi-manual-ssid')).toBeUndefined();
        expect(onConnectionRequest).not.toHaveBeenCalled();
    });

    test('a secured network joins with its password', async () => {
        const { onConnectionRequest } = renderSettings();
        await chooseNetwork('Other…');
        await changeInput(manualInput('ssid'), 'HiddenNet');
        await changeInput(manualInput('security'), pb.EncryptionType.WPA2);
        await changeInput(manualInput('password'), 'hunter22');

        await clickButton('Join');

        expect(onConnectionRequest).toHaveBeenCalledWith('HiddenNet', pb.EncryptionType.WPA2, 'hunter22');
    });

    test('a password typed before switching to no security is not sent', async () => {
        const { onConnectionRequest } = renderSettings();
        await chooseNetwork('Other…');
        await changeInput(manualInput('ssid'), 'OpenNet');
        await changeInput(manualInput('security'), pb.EncryptionType.WPA2);
        await changeInput(manualInput('password'), 'hunter22');
        await changeInput(manualInput('security'), pb.EncryptionType.NONE);

        await clickButton('Join');

        expect(onConnectionRequest).toHaveBeenCalledWith('OpenNet', pb.EncryptionType.NONE, '');
    });

    test('a refused join leaves what was typed editable for another try', async () => {
        renderSettings(async () => false);
        await chooseNetwork('Other…');
        await changeInput(manualInput('ssid'), 'HiddenNet');
        await changeInput(manualInput('security'), pb.EncryptionType.WPA2);
        await changeInput(manualInput('password'), 'hunter22');

        await clickButton('Join');

        expect(manualInput('ssid')).toHaveProperty('value', 'HiddenNet');
        expect(manualInput('password')).toHaveProperty('value', 'hunter22');
        expect(manualInput('ssid')).toHaveProperty('disabled', false);
    });
});

describe('<SectionSettings /> joining a listed secured network', () => {
    afterEach(cleanup);

    test('an empty password is refused under its field', async () => {
        const { onConnectionRequest } = renderSettings();
        await chooseNetwork('HomeNet');

        await clickButton('Connect');

        expect(errorUnder('wifi-password-entry')).toBe('Missing value!');
        expect(onConnectionRequest).not.toHaveBeenCalled();
    });

    test('the closing password dialog still names its network', async () => {
        renderSettings();
        await chooseNetwork('HomeNet');
        const dialog = dialogTitled('Password for HomeNet');

        await act(async () => {
            fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
        });

        expect(within(dialog).getByRole('heading', { name: 'Password for HomeNet' })).not.toBeNull();
    });

    test('a cancelled password dialog does not keep the password', async () => {
        renderSettings();
        await chooseNetwork('HomeNet');
        const dialog = dialogTitled('Password for HomeNet');
        await changeInput(passwordInput(), 'hunter22');

        await act(async () => {
            fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
        });

        expect(passwordInput()).toHaveProperty('value', '');
    });

    test('Escape leaves a running join alone', async () => {
        let settle: (joined: boolean) => void = () => {
            throw new Error('BUG: the join was never requested');
        };
        const { onConnectionRequestCancel } = renderSettings(() => new Promise(resolve => (settle = resolve)));
        await chooseNetwork('HomeNet');
        await changeInput(passwordInput(), 'hunter22');
        await clickButton('Connect');

        await act(async () => {
            fireEvent.keyDown(document, { key: 'Escape' });
        });
        await act(async () => settle(false));

        expect(onConnectionRequestCancel).not.toHaveBeenCalled();
        expect(passwordInput()).toHaveProperty('value', 'hunter22');
    });
});

describe('<SectionSettings /> network dropdown', () => {
    afterEach(cleanup);

    test('shows the network being joined and stays read-only until the join settles', async () => {
        let settle: (joined: boolean) => void = () => {
            throw new Error('BUG: the join was never requested');
        };
        renderSettings(() => new Promise(resolve => (settle = resolve)));
        await chooseNetwork('Other…');
        await changeInput(manualInput('ssid'), 'CafeNet');

        await clickButton('Join');

        expect(dropdown().textContent).toContain('CafeNet');
        expect(dropdown().getAttribute('aria-disabled')).toBe('true');

        await act(async () => settle(true));

        expect(dropdown().textContent).not.toContain('CafeNet');
        expect(dropdown().getAttribute('aria-disabled')).toBeNull();
    });

    test('draws no network icons on the "Other…" entry', async () => {
        renderSettings();

        await act(async () => {
            fireEvent.click(dropdown());
        });

        expect(screen.getByRole('option', { name: 'HomeNet' }).querySelector('svg')).not.toBeNull();
        expect(screen.getByRole('option', { name: 'Other…' }).querySelector('svg')).toBeNull();
    });
});
