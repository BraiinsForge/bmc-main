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
import { useEffect, useRef } from 'react';
import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';

import { FileUploader } from './FileUploader';
import { assertUnreachable } from '@/lib/ts';

type Preset = 'idle' | 'dragOver' | 'chosen' | 'invalid';

function drag(zone: HTMLElement, type: 'dragover' | 'drop', name: string): void {
    const dataTransfer = new DataTransfer();
    dataTransfer.items.add(new File(['x'.repeat(1500)], name));
    zone.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer }));
}

function State({ title, preset }: { title: string; preset: Preset }) {
    const id = `uploader-${preset}`;
    const ref = useRef<HTMLDivElement>(null);

    useEffect(() => {
        const zone = ref.current?.querySelector<HTMLElement>(`#${id}`);
        if (!zone) throw new Error(`BUG: #${id} is rendered`);
        switch (preset) {
            case 'idle':
                return;
            case 'dragOver':
                return drag(zone, 'dragover', 'widget-example.nar');
            case 'chosen':
                return drag(zone, 'drop', 'widget-example.nar');
            case 'invalid':
                return drag(zone, 'drop', 'widget-example.zip');
            default:
                return assertUnreachable(preset, 'FileUploader story preset');
        }
    }, [id, preset]);

    return (
        <div ref={ref} className="ui-box" style={{ width: 480 }}>
            <h5 children={title} />
            <FileUploader id={id} accept={['.nar']} onChange={action(`onChange (${preset})`)} />
        </div>
    );
}

export default {
    title: 'components/FileUploader',
    component: FileUploader,
} satisfies Meta<typeof FileUploader>;

export const States = () => (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 16, padding: 16 }}>
        <State title="Idle" preset="idle" />
        <State title="Drag over" preset="dragOver" />
        <State title="Chosen" preset="chosen" />
        <State title="Invalid" preset="invalid" />
    </div>
);
