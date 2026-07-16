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

import { UpgradeStepper as Component } from './UpgradeStepper';
import { fixtures } from './UpgradeStepper.fixtures';

export default {
    title: 'settings/components/UpgradeStepper',
    component: Component,
} satisfies Meta<typeof Component>;

// Every stepper state on one screen — the fixtures the spec also asserts.
export function UpgradeStepper() {
    return (
        <div style={{ display: 'flex', flexFlow: 'row wrap', padding: 16, gap: 16 }}>
            {Object.entries(fixtures).map(([title, props]) => (
                <section key={title}>
                    <small
                        children={title}
                        style={{
                            display: 'flex',
                            placeSelf: 'start',
                            padding: 8,
                            margin: 8,
                            backgroundColor: '#666',
                            fontSize: 12,
                        }}
                    />
                    <div className="ui-box" style={{ maxWidth: 640 }}>
                        <Component {...props} />
                    </div>
                </section>
            ))}
        </div>
    );
}
