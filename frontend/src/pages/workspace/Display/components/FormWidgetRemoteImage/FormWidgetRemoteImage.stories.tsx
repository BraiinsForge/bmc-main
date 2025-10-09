import { useState, useCallback } from 'react';
import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import {
    FormWidgetRemoteImage as Component,
    type FormWidgetRemoteImageProps,
} from 'src/pages/workspace/Display/components/FormWidgetRemoteImage/FormWidgetRemoteImage.tsx';

export default {
    title: 'display/components/FormWidgetRemoteImage',
    component: Component,
} satisfies Meta<FormWidgetRemoteImageProps>;

function Demo() {
    const isInvalid: boolean = true;
    const err = (text: string): null | string => (isInvalid ? text : null);

    type StateProps = Exclude<keyof FormWidgetRemoteImageProps, 'isOpen' | 'isEdit' | 'onClose' | 'onSubmit' | 'error'>;
    const handleChange = useCallback(<Key extends StateProps>(key: Key) => {
        return (value: any) => {
            setArgs(prev => ({ ...prev, [key]: { ...prev[key], value } }));
        };
    }, []);
    const [args, setArgs] = useState<FormWidgetRemoteImageProps>({
        isOpen: true,
        isEdit: false,
        onClose: action('onClose'),
        error: err('Global error'),

        widgetSize: {
            value: pb.WidgetSize.MEDIUM,
            disabled: false,
            error: err('The bliss is an ultimate believer!'),
            onChange: handleChange('widgetSize'),
            options: [pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE],
        },
        url: {
            value: 'https://picsum.photos/200',
            disabled: false,
            error: err('Ususs sunt lumens de grandis resistentia!'),
            onChange: handleChange('url'),
        },
        refreshDurationSec: {
            value: 60,
            disabled: false,
            error: err('Ususs sunt lumens de grandis resistentia!'),
            onChange: handleChange('refreshDurationSec'),
        },
    });

    return <Component {...args} style={{ padding: 18, backgroundColor: 'var(--cds-layer-01)' }} />;
}

export function FormWidgetRemoteImage() {
    return <Demo />;
}
