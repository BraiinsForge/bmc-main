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

import { afterEach, describe, expect, test } from '@rstest/core';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';

import * as pb from '@/proto';
import * as fn from '../../fn';
import { CombinedSceneView } from './CombinedSceneView';

afterEach(cleanup);

const noop = () => undefined;

function placed(id: string, uid: string, col: number): pb.Widget {
    return pb.create(pb.WidgetSchema, {
        id,
        position: fn.pos(0, col),
        size: pb.WidgetSize.SMALL,
        config: pb.create(pb.WidgetConfigSchema, { widgetUid: uid, params: {} }),
    });
}

function manifest(uid: string, name: string, subname?: string): [string, pb.WidgetManifest] {
    return [uid, pb.create(pb.WidgetManifestSchema, { uid, name, subname, supportedSizes: [pb.WidgetSize.SMALL] })];
}

function renderScene(widgets: pb.Widget[]) {
    render(
        <IntlProvider locale="en">
            <CombinedSceneView
                widgets={widgets}
                manifests={
                    new Map([
                        manifest('clock', 'Clock'),
                        manifest('weather', 'Weather'),
                        manifest('geek', 'Miner Info', 'Geek'),
                        manifest('mining', 'Miner Info', 'Mining'),
                    ])
                }
                onWidgetMove={noop}
                onWidgetAdd={noop}
                onWidgetEdit={noop}
                onWidgetRemove={noop}
            />
        </IntlProvider>,
    );
}

describe('CombinedSceneView', () => {
    test.each(['Move', 'Edit', 'Remove'])('each placed widget has a %s button named after it', action => {
        renderScene([placed('w1', 'clock', 0), placed('w2', 'weather', 1)]);
        expect(screen.getByRole('button', { name: `${action} Clock` })).toBeTruthy();
        expect(screen.getByRole('button', { name: `${action} Weather` })).toBeTruthy();
    });

    test('widgets of the same name are told apart by their subname', () => {
        renderScene([placed('w1', 'geek', 0), placed('w2', 'mining', 1)]);
        expect(screen.getByRole('button', { name: 'Remove Miner Info Geek' })).toBeTruthy();
        expect(screen.getByRole('button', { name: 'Remove Miner Info Mining' })).toBeTruthy();
    });

    test('a widget picked up is announced with its subname', async () => {
        renderScene([placed('w1', 'geek', 0)]);
        const handle = screen.getByRole('button', { name: 'Move Miner Info Geek' });

        fireEvent.pointerDown(handle, { isPrimary: true, button: 0, clientX: 0, clientY: 0 });

        const announced = () =>
            [...document.querySelectorAll('[id^="DndLiveRegion"]')].map(region => region.textContent).join(' ');
        await waitFor(() => expect(announced()).toContain('Picked up Miner Info Geek.'));
        expect(screen.queryByRole('button', { name: 'Move' }), 'the dragged copy keeps its name').toBeNull();
    });

    test('a widget without its manifest is named as its card reads', () => {
        renderScene([placed('w1', 'uninstalled', 0)]);
        expect(screen.getByRole('button', { name: 'Remove N/A' })).toBeTruthy();
    });

    test('a free slot has a button named Add Widget', () => {
        renderScene([placed('w1', 'clock', 0)]);
        expect(screen.getAllByRole('button', { name: 'Add Widget' }).length).toBeGreaterThan(0);
    });
});
