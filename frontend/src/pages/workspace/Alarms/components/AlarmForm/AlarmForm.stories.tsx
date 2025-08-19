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

function Demo() {
    const [time, setTime] = useState<string>('11:55');
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
export function AlarmForm() {
    return <Demo />;
}
