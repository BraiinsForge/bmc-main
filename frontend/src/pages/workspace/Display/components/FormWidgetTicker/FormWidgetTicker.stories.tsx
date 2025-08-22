import { useState, useCallback } from 'react';
import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import { FormWidgetTicker as Component, type FormWidgetTickerProps } from './FormWidgetTicker';

export default {
    title: 'display/components/FormWidgetTicker',
    component: Component,
} satisfies Meta<FormWidgetTickerProps>;

function Demo() {
    type StateProps = Exclude<keyof FormWidgetTickerProps, 'isOpen' | 'isEdit' | 'onClose' | 'onSubmit' | 'error'>;
    const handleChange = useCallback(<Key extends StateProps>(key: Key) => {
        return (value: any) => {
            setArgs(prev => ({ ...prev, [key]: { ...prev[key], value } }));
        };
    }, []);
    const [args, setArgs] = useState<FormWidgetTickerProps>({
        isOpen: true,
        isEdit: false,
        onClose: action('onClose'),
        error: 'Global error',

        widgetSize: {
            value: pb.WidgetSize.MEDIUM,
            disabled: false,
            error: 'The bliss is an ultimate believer!',
            onChange: handleChange('widgetSize'),
            options: [pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE],
        },
        timeFrame: {
            value: pb.TickerBtcWidget_TimeFrame.DAY_1,
            disabled: false,
            error: 'Pol, barcas!',
            onChange: handleChange('timeFrame'),
            options: [],
        },
    });

    return <Component {...args} style={{ padding: 18, backgroundColor: 'var(--cds-layer-01)' }} />;
}

export function FormWidgetTicker() {
    return <Demo />;
}
