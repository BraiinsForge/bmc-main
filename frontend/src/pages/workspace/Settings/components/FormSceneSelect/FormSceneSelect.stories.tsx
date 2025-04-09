import type { Meta } from '@storybook/react';
import { action } from '@storybook/addon-actions';
import { FormSceneSelect as Component, type FormSceneSelectProps } from './FormSceneSelect';

export default {
    title: 'settings/components/FormSceneSelect',
    component: Component,
} satisfies Meta<FormSceneSelectProps>;

export function FormSceneSelect() {
    return <Component onClick={action('onClick')} />;
}
