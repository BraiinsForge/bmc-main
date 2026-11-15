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

import * as pb from '@/proto';
import { AlarmDialog as Component, type AlarmDialogProps } from './AlarmDialog';

export default {
    title: 'Alarms/components/AlarmDialog',
    component: Component,
} satisfies Meta<AlarmDialogProps>;

const sounds: pb.SoundInfo[] = [
    pb.create(pb.SoundInfoSchema, { id: 'birds', name: 'Birds' }),
    pb.create(pb.SoundInfoSchema, { id: 'chimes', name: 'Chimes' }),
];

const noop = () => {};

interface DemoProps {
    alarmID?: null | string;
    time?: string;
    errors?: string[];
    timeError?: string;
    soundError?: string;
}

function Demo({
    alarmID = '0b0e7a3c-5c2f-4f0e-9a51-6f1f3c1d2e4a',
    time = '07:30',
    errors,
    timeError,
    soundError,
}: DemoProps) {
    return (
        <Component
            open
            alarmID={alarmID}
            errors={errors}
            form={{
                time: { value: time, error: timeError },
                timeFormat: pb.TimeFormat.TIME_FORMAT_24_HOUR,
                name: { value: 'Morning' },
                repeat: { value: [pb.Weekday.MONDAY, pb.Weekday.FRIDAY] },
                sound: { value: 'birds', options: sounds, error: soundError },
                snoozeEnabled: { value: true },
                snoozeLimit: { value: pb.SnoozeLimit.SNOOZE_LIMIT_FOREVER },
                snoozeDuration: { value: pb.SnoozeDuration.SNOOZE_DURATION_5_MINUTES },
            }}
            onSubmit={noop}
            onClose={noop}
            onDelete={noop}
        />
    );
}

const refused = 'Some of the fields are invalid.';

export function Add() {
    return <Demo alarmID={null} />;
}
export function ConnectionLost() {
    return <Demo errors={['Failed to save the alarm!']} />;
}

export function SnoozeRejected() {
    return <Demo errors={[refused, 'Missing value!']} />;
}
export function SoundRejected() {
    return <Demo errors={[refused]} soundError="Invalid sound!" />;
}
export function InvalidTime() {
    return <Demo time="7:5" errors={[]} timeError="Invalid time format" />;
}
