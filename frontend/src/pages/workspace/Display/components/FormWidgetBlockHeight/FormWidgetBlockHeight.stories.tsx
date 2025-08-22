import { useState, useCallback } from 'react';
import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import { FormWidgetBlockHeight as Component, type FormWidgetBlockHeightProps } from './FormWidgetBlockHeight';

export default {
    title: 'display/components/FormWidgetBlockHeight',
    component: Component,
} satisfies Meta<FormWidgetBlockHeightProps>;

function Demo() {
    type StateProps = Exclude<keyof FormWidgetBlockHeightProps, 'isOpen' | 'isEdit' | 'onClose' | 'onSubmit' | 'error'>;
    const handleChange = useCallback(<Key extends StateProps>(key: Key) => {
        return (value: any) => {
            setArgs(prev => ({ ...prev, [key]: { ...prev[key], value } }));
        };
    }, []);
    const [args, setArgs] = useState<FormWidgetBlockHeightProps>({
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
    });

    return <Component {...args} style={{ padding: 18, backgroundColor: 'var(--cds-layer-01)' }} />;
}

export function FormWidgetBlockHeight() {
    return <Demo />;
}
