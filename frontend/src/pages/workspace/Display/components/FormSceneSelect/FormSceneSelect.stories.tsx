import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';
import { FormSceneSelect as Component, type FormSceneSelectProps } from './FormSceneSelect';

export default {
    title: 'display/components/FormSceneSelect',
    component: Component,
} satisfies Meta<FormSceneSelectProps>;

export function FormSceneSelect() {
    return <Component onClick={action('onClick')} />;
}
