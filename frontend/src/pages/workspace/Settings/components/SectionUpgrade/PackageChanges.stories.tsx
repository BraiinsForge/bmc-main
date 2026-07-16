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

import type { Meta } from '@storybook/react';

import type * as pb from '@/proto';
import { PackageChanges as Component } from './PackageChanges';

const changes: pb.PackageChange[] = [
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'bmc',
        category: 'core',
        versionFrom: '24.08.1',
        versionTo: '24.09.4',
        changelog: '## Coordinator\n\n- faster scene switching\n- fixed a rare memory leak on wake',
    },
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'clock-widget',
        category: 'widget',
        versionFrom: '1.4.0',
        versionTo: '1.5.0',
        changelog: '- new analog face\n- DST edge-case fix',
    },
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'weather-widget',
        category: 'widget',
        versionTo: '2.0.0',
    },
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'braiins-cli',
        category: 'dev',
        versionFrom: '1.1.0',
        versionTo: '1.2.0',
        changelog: 'Minor CLI polish.',
    },
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'nix-support',
        versionTo: '0.3.0',
    },
    {
        $typeName: 'braiins.bmc.web.PackageChange',
        name: 'legacy-ticker',
        category: 'widget',
        versionFrom: '0.4.2',
    },
];

export default {
    title: 'settings/components/PackageChanges',
    component: Component,
    decorators: [
        Story => (
            <div className="ui-box" style={{ maxWidth: 560 }}>
                <Story />
            </div>
        ),
    ],
} satisfies Meta<typeof Component>;

export function PackageChanges() {
    return <Component changes={changes} />;
}
