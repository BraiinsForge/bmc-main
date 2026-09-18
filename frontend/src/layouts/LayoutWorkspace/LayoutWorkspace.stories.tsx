// Copyright (C) 2025  Braiins Systems s.r.o.
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

import type { ReactNode } from 'react';
import type { Meta, StoryObj } from '@storybook/react';
import { store } from '@/store';
import type { Brand } from '@/lib/brand';
import type { Capabilities } from '@/lib/system';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import headerLogoUrl from '@/components/images/logos/header.svg?url';
import { LayoutWorkspace as Component } from './LayoutWorkspace';

// Mirrors boser's shipped `/var/brand.js`, which the header consumes on a managed device.
const BRAND: Brand = {
    name: 'Braiins OS',
    logo: { header: { src: headerLogoUrl, style: { padding: 14 } } },
    links: {
        products: [
            { href: 'https://braiins.com', text: 'Braiins.com' },
            { href: 'https://braiins.com/pool', text: 'Braiins Pool' },
            { href: 'https://braiins.com/os-firmware', text: 'Braiins\xa0OS', isActive: true },
            { href: 'https://braiins.com/toolbox', text: 'Braiins Toolbox' },
        ],
    },
};

// The layout reads the store the app seeds from `system.js` at boot; Storybook has no such script.
function onDevice(capabilities: Capabilities, brand: null | Brand) {
    return (Story: () => ReactNode) => {
        store.setHardwareCapabilities(capabilities);
        store.setBrand(brand);
        return <Story />;
    };
}

const children =
    'Lorem ipsum dolor sit amet, consectetur adipisicing elit. Assumenda atque, consequatur cumque dolores ' +
    'dolorum in minima molestiae natus, officiis, omnis pariatur quisquam tempore ullam voluptate voluptatem.';

export default {
    title: 'layouts/LayoutWorkspace',
    component: Component,
    parameters: { layout: 'fullscreen' },
    args: { children },
} satisfies Meta<typeof Component>;

type Story = StoryObj<typeof Component>;

export const Deck: Story = {
    decorators: [onDevice(deckCapabilities(), null)],
};

export const BoserManaged: Story = {
    decorators: [onDevice(deckCapabilities({ boserManaged: true, ethernetSupported: true }), BRAND)],
};

// The flags `bmc-platform` reports for BMM101.
export const Bmm101: Story = {
    decorators: [
        onDevice(
            deckCapabilities({
                combinedScenesSupported: false,
                ethernetSupported: true,
                miningSupported: true,
                soundSupported: false,
                ledSupported: false,
                alarmSupported: false,
                boserManaged: true,
                productName: 'Braiins Mini Miner',
            }),
            BRAND,
        ),
    ],
};
