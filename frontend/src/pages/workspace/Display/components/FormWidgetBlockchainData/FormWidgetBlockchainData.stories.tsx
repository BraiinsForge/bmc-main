import { useState, useCallback } from 'react';
import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import { FormWidgetBlockchainData as Component, type FormWidgetBlockchainDataProps } from './FormWidgetBlockchainData';

export default {
    title: 'display/components/FormWidgetBlockchainData',
    component: Component,
} satisfies Meta<FormWidgetBlockchainDataProps>;

function Demo() {
    type StateProps = Exclude<
        keyof FormWidgetBlockchainDataProps,
        'isOpen' | 'isEdit' | 'onClose' | 'onSubmit' | 'error'
    >;
    const handleChange = useCallback(<Key extends StateProps>(key: Key) => {
        return (value: any) => {
            setArgs(prev => ({ ...prev, [key]: { ...prev[key], value } }));
        };
    }, []);
    const [args, setArgs] = useState<FormWidgetBlockchainDataProps>({
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
    });

    return <Component {...args} style={{ padding: 18, backgroundColor: 'var(--cds-layer-01)' }} />;
}

export function FormWidgetBlockchainData() {
    return <Demo />;
}
