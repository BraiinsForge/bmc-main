import type { Meta } from '@storybook/react';
import { action } from '@storybook/addon-actions';
import { SectionSecurity as Component, type SectionSecurityProps } from './SectionSecurity';

export default {
    title: 'settings/components/SectionSecurity',
    component: Component,
    args: {
        hasPassword: true,
        onPasswordChange: async d => action('onPasswordChange')(d),
        onPasswordRemove: async d => action('onPasswordRemove')(d),
        onPasswordCreate: async d => action('onPasswordCreate')(d),
        dataCollection: {
            value: true,
            disabled: false,
            onChange: action('dataCollection.onChange'),
        },
    } satisfies SectionSecurityProps,
} satisfies Meta<SectionSecurityProps>;

export function SectionSecurity(args: SectionSecurityProps) {
    return <Component {...args} />;
}
