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
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';

import * as pb from '@/proto';

import { SectionSettings } from './SectionSettings';

const SECURED_NETWORK = pb.create(pb.WifiNetworkSchema, {
    ssid: 'HomeNet',
    encryptionType: pb.EncryptionType.WPA2,
});

function renderSettings() {
    const onConnectionRequest = rstest.fn(async () => true);
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
                    activeNetwork: { value: null, onChange: () => {}, onConnectionRequest },
                    availableNetworks: { isLoading: false, options: [SECURED_NETWORK], onRefresh: () => {} },
                }}
            />
        </IntlProvider>,
    );
    return { onConnectionRequest };
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
});
