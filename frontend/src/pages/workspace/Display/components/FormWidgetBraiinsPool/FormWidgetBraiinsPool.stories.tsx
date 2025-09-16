import { useState, useCallback } from 'react';
import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import * as gen from '@/mocks';
import { formatISO as formatDateIso } from 'date-fns';
import { FormWidgetBraiinsPool as Component, type FormWidgetBraiinsPoolProps } from './FormWidgetBraiinsPool';

export default {
    title: 'display/components/FormWidgetBraiinsPool',
    component: Component,
} satisfies Meta<FormWidgetBraiinsPoolProps>;

const accounts = gen.arrayOf<pb.Account>(6, () => {
    return pb.create(pb.AccountSchema, {
        id: gen.uuid(),
        accountType: pb.AccountType.BRAIINSPOOL,
        accountName: gen.lorem.generateWords(2),
        authentication: { $typeName: 'braiins.bmc.web.Authentication', value: { case: 'apiKey', value: gen.uuid() } },
        createdAt: formatDateIso(new Date()),
    });
});

function Demo() {
    type StateProps = Exclude<keyof FormWidgetBraiinsPoolProps, 'isOpen' | 'isEdit' | 'onClose' | 'onSubmit' | 'error'>;
    const handleChange = useCallback(<Key extends StateProps>(key: Key) => {
        return (value: any) => {
            setArgs(prev => ({ ...prev, [key]: { ...prev[key], value } }));
        };
    }, []);
    const [args, setArgs] = useState<FormWidgetBraiinsPoolProps>({
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
        accountId: {
            value: accounts[0].id,
            disabled: false,
            error: 'The bliss is an ultimate believer!',
            onChange: handleChange('accountId'),
            options: accounts,
        },
        sceneStyle: {
            value: pb.BraiinsPoolWidget_BraiinsPoolStyle.OVERVIEW,
            disabled: false,
            error: 'Ususs sunt lumens de grandis resistentia!',
            onChange: handleChange('sceneStyle'),
        },
        timeFrame: {
            value: pb.BraiinsPoolWidget_TimeFrame.HOUR_4,
            disabled: false,
            error: 'Ususs sunt lumens de grandis resistentia!',
            onChange: handleChange('timeFrame'),
        },
    });

    return <Component {...args} style={{ padding: 18, backgroundColor: 'var(--cds-layer-01)' }} />;
}

export function FormWidgetBraiinsPool() {
    return <Demo />;
}
