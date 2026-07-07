import type { Meta } from '@storybook/react';

import { UpgradeStepper as Component } from './UpgradeStepper';
import { fixtures } from './UpgradeStepper.fixtures';

export default {
    title: 'settings/components/UpgradeStepper',
    component: Component,
} satisfies Meta<typeof Component>;

// Every stepper state on one screen — the fixtures the spec also asserts.
export function UpgradeStepper() {
    return (
        <div style={{ display: 'flex', flexFlow: 'row wrap', padding: 16, gap: 16 }}>
            {Object.entries(fixtures).map(([title, props]) => (
                <section key={title}>
                    <small
                        children={title}
                        style={{
                            display: 'flex',
                            placeSelf: 'start',
                            padding: 8,
                            margin: 8,
                            backgroundColor: '#666',
                            fontSize: 12,
                        }}
                    />
                    <div className="ui-box" style={{ maxWidth: 640 }}>
                        <Component {...props} />
                    </div>
                </section>
            ))}
        </div>
    );
}
