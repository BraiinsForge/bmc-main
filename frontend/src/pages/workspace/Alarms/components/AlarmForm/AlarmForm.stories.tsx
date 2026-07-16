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

import { useState } from 'react';
import type { Meta } from '@storybook/react';

import { AlarmForm as Component, type AlarmFormProps } from './AlarmForm';
import * as get from '@/mocks';
import * as pb from '@/proto';

export default {
    title: 'Alarms/components/AlarmForm',
    component: Component,
} satisfies Meta<AlarmFormProps>;

const sounds: pb.SoundInfo[] = get.arrayOf<pb.SoundInfo>(5, () => {
    return pb.create(pb.SoundInfoSchema, { id: get.uuid(), name: get.hostname(2, '') });
});

function Demo({ timeFormat, initialTime }: { timeFormat: pb.TimeFormat; initialTime: string }) {
    const [time, setTime] = useState<string>(initialTime);
    const [name, setName] = useState<string>('My Alarm');
    const [repeat, setRepeat] = useState<pb.Weekday[]>([pb.Weekday.MONDAY, pb.Weekday.WEDNESDAY]);
    const [sound, setSound] = useState<pb.SoundInfo['id'] | null>(() => get.randomItem(sounds)?.id);

    const [snoozeEnabled, setSnoozeEnabled] = useState<boolean>(true);
    const [snoozeLimit, setSnoozeLimit] = useState<pb.SnoozeLimit>(pb.SnoozeLimit.SNOOZE_LIMIT_FOREVER);
    const [snoozeDuration, setSnoozeDuration] = useState<pb.SnoozeDuration>(
        pb.SnoozeDuration.SNOOZE_DURATION_5_MINUTES,
    );

    return (
        <div className="ui-box">
            <Component
                time={{ value: time, onChange: setTime }}
                timeFormat={timeFormat}
                name={{ value: name, onChange: setName }}
                repeat={{ value: repeat, onChange: setRepeat }}
                sound={{ value: sound, options: sounds, onChange: setSound }}
                snoozeEnabled={{ value: snoozeEnabled, onChange: setSnoozeEnabled }}
                snoozeLimit={{ value: snoozeLimit, onChange: setSnoozeLimit }}
                snoozeDuration={{ value: snoozeDuration, onChange: setSnoozeDuration }}
            />
        </div>
    );
}
export function AlarmForm24h() {
    return <Demo timeFormat={pb.TimeFormat.TIME_FORMAT_24_HOUR} initialTime="11:55" />;
}
export function AlarmForm12h() {
    return <Demo timeFormat={pb.TimeFormat.TIME_FORMAT_12_HOUR} initialTime="12:00 PM" />;
}
