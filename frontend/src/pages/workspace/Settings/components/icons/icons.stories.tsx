import type { Meta } from '@storybook/react';
import * as X from './index';

export default {
    title: 'settings/components/Icons',
} satisfies Meta;

export function Icons() {
    return (
        <div
            style={{ display: 'flex', flexDirection: 'row', gap: 8, padding: 8, width: 600 }}
            children={Object.values(X).map((Icon, i) => <Icon key={i} size={64} />)}
        />
    );
}
