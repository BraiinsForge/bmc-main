import { useState, useCallback } from 'react';
import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import { FormWidgetClock as Component, type FormWidgetClockProps } from './FormWidgetClock';

export default {
    title: 'display/components/FormWidgetClock',
    component: Component,
} satisfies Meta<FormWidgetClockProps>;

function Demo() {
    type StateProps = Exclude<keyof FormWidgetClockProps, 'isOpen' | 'isEdit' | 'onClose' | 'onSubmit'>;
    const handleChange = useCallback(<Key extends StateProps>(key: Key) => {
        return (value: any) => {
            setArgs(prev => ({ ...prev, [key]: { ...prev[key], value } }));
        };
    }, []);
    const [args, setArgs] = useState<FormWidgetClockProps>({
        isOpen: true,
        isEdit: false,
        onClose: action('onClose'),

        widgetSize: {
            value: pb.WidgetSize.MEDIUM,
            disabled: false,
            error: 'The bliss is an ultimate believer!',
            onChange: handleChange('widgetSize'),
            options: [pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE],
        },
        clockStyle: {
            value: pb.ClockWidget_ClockStyle.DIGITAL,
            disabled: false,
            error: 'Vision doesn’t spiritually know any ego!',
            onChange: handleChange('clockStyle'),
        },
        fontStyle: {
            value: pb.FontStyle.MEDIUM,
            disabled: false,
            error: 'The bliss is an ultimate believer!',
            onChange: handleChange('fontStyle'),
        },
        showDate: {
            value: true,
            disabled: false,
            error: 'Ususs sunt lumens de grandis resistentia!',
            onChange: handleChange('showDate'),
        },
        showSeconds: {
            value: false,
            disabled: false,
            error: 'Idoleum de raptus glos, desiderium hibrida!',
            onChange: handleChange('showSeconds'),
        },
        showTimezone: {
            value: true,
            disabled: false,
            error: 'Man, vision and a playful great unknown!',
            onChange: handleChange('showTimezone'),
        },
        timezone: {
            value: 'Europe/Berlin',
            disabled: false,
            error: 'Pol, barcas!',
            onChange: handleChange('timezone'),
            options: [],
        },

        // showWeather: {
        //     value: true,
        //     disabled: false,
        //     error: 'Frondator de talis racana, reperire rector!',
        //     onChange: handleChange('showWeather'),
        // },
        // weatherLocation: {
        //     value: 'Berlin',
        //     disabled: false,
        //     error: 'The bliss is an ultimate believer!',
        //     onChange: handleChange('weatherLocation'),
        // },
    });

    return <Component {...args} style={{ padding: 18, backgroundColor: 'var(--cds-layer-01' }} />;
}

export function FormWidgetClock() {
    return <Demo />;
}
