import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';
import { Welcome as Component } from './Welcome';

export default {
    title: 'init/Welcome',
    component: Component,
} satisfies Meta;

export function Welcome() {
    return <Component onNext={action('onNext')} />;
}
